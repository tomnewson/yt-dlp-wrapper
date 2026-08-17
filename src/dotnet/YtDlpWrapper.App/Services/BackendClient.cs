using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text.Json;

namespace YtDlpWrapper.Services;

public sealed class BackendClient(IPlatformServices platform) : IBackendClient
{
    private const int ProtocolVersion = 1;
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);
    private readonly ConcurrentDictionary<string, TaskCompletionSource<JsonElement>> _pending = [];
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private Process? _process;
    private Task? _stdoutTask;
    private Task? _stderrTask;
    private bool _stopping;

    public event Action<BackendEvent>? EventReceived;
    public event Action<string?>? BackendExited;

    public Task StartAsync(CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (_process is { HasExited: false })
        {
            return Task.CompletedTask;
        }

        _process?.Dispose();
        _process = null;

        var startInfo = platform.CreateBackendStartInfo();
        if (!File.Exists(startInfo.FileName))
        {
            throw new FileNotFoundException("The Rust backend is missing from the application folder.", startInfo.FileName);
        }

        _stopping = false;
        _process = new Process { StartInfo = startInfo, EnableRaisingEvents = true };
        if (!_process.Start())
        {
            throw new InvalidOperationException("The Rust backend could not be started.");
        }

        _stdoutTask = ReadStdoutAsync(_process);
        _stderrTask = DrainStderrAsync(_process);
        _ = ObserveExitAsync(_process);
        return Task.CompletedTask;
    }

    public async Task<JsonElement> SendAsync(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default)
    {
        var process = _process;
        if (process is null || process.HasExited)
        {
            throw new InvalidOperationException("The Rust backend is not running.");
        }

        var requestId = Guid.NewGuid().ToString("D");
        var completion = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (!_pending.TryAdd(requestId, completion))
        {
            throw new InvalidOperationException("Could not allocate a backend request ID.");
        }

        using var registration = cancellationToken.Register(() =>
        {
            if (_pending.TryRemove(requestId, out var pending))
            {
                pending.TrySetCanceled(cancellationToken);
            }
        });

        var request = new
        {
            protocolVersion = ProtocolVersion,
            kind = "request",
            requestId,
            method,
            @params = parameters ?? new { },
        };

        try
        {
            var line = JsonSerializer.Serialize(request, JsonOptions);
            await _writeLock.WaitAsync(cancellationToken);
            try
            {
                await process.StandardInput.WriteLineAsync(line.AsMemory(), cancellationToken);
                await process.StandardInput.FlushAsync(cancellationToken);
            }
            finally
            {
                _writeLock.Release();
            }

            return await completion.Task;
        }
        catch
        {
            _pending.TryRemove(requestId, out _);
            throw;
        }
    }

    public async Task StopAsync()
    {
        var process = _process;
        if (process is null)
        {
            return;
        }

        _stopping = true;
        if (!process.HasExited)
        {
            try
            {
                using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                await SendAsync("shutdown", cancellationToken: timeout.Token);
                await process.WaitForExitAsync(timeout.Token);
            }
            catch (Exception)
            {
                if (!process.HasExited)
                {
                    try
                    {
                        process.Kill(entireProcessTree: true);
                        using var killTimeout = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                        await process.WaitForExitAsync(killTimeout.Token);
                    }
                    catch (Exception)
                    {
                        // The process may exit between the state check and kill.
                    }
                }
            }
        }

        try
        {
            process.StandardInput.Close();
        }
        catch (Exception)
        {
            // The backend may have already closed its redirected input stream.
        }

        var readers = new[] { _stdoutTask, _stderrTask }
            .Where(task => task is not null)
            .Cast<Task>()
            .ToArray();
        if (readers.Length > 0)
        {
            await SuppressFailure(
                Task.WhenAll(readers).WaitAsync(TimeSpan.FromSeconds(1)));
        }

        process.Dispose();
        _process = null;
        FailPending(new InvalidOperationException("The Rust backend has stopped."));
    }

    public async ValueTask DisposeAsync()
    {
        await StopAsync();
        _writeLock.Dispose();
    }

    private async Task ReadStdoutAsync(Process process)
    {
        try
        {
            while (await process.StandardOutput.ReadLineAsync() is { } line)
            {
                HandleMessage(line);
            }
        }
        catch (Exception error)
        {
            FailPending(error);
        }
    }

    private static async Task DrainStderrAsync(Process process)
    {
        while (await process.StandardError.ReadLineAsync() is not null)
        {
            // Diagnostics are written by Rust to its redacted rolling log. Do not
            // mirror stderr into the UI process where request data could leak.
        }
    }

    private void HandleMessage(string line)
    {
        JsonDocument document;
        try
        {
            document = JsonDocument.Parse(line);
        }
        catch (JsonException error)
        {
            FailPending(new InvalidDataException("The backend emitted malformed JSON.", error));
            return;
        }

        using (document)
        {
            var root = document.RootElement;
            if (!root.TryGetProperty("protocolVersion", out var version) || version.GetInt32() != ProtocolVersion)
            {
                FailPending(new InvalidDataException("The backend protocol version is incompatible."));
                return;
            }

            var kind = root.GetProperty("kind").GetString();
            if (kind == "event")
            {
                EventReceived?.Invoke(new BackendEvent(
                    root.GetProperty("operationId").GetString() ?? string.Empty,
                    root.GetProperty("event").GetString() ?? string.Empty,
                    root.GetProperty("data").Clone()));
                return;
            }

            if (kind != "response")
            {
                return;
            }

            var requestId = root.GetProperty("requestId").GetString() ?? string.Empty;
            if (!_pending.TryRemove(requestId, out var completion))
            {
                return;
            }

            if (root.GetProperty("ok").GetBoolean())
            {
                completion.TrySetResult(root.TryGetProperty("result", out var result) ? result.Clone() : default);
            }
            else
            {
                var error = root.GetProperty("error").Deserialize<BackendWireError>(JsonOptions)
                    ?? new BackendWireError("unknown", "The backend request failed.", null);
                completion.TrySetException(new BackendRequestException(error));
            }
        }
    }

    private async Task ObserveExitAsync(Process process)
    {
        int exitCode;
        try
        {
            await process.WaitForExitAsync();
            exitCode = process.ExitCode;
        }
        catch (Exception) when (_stopping)
        {
            return;
        }

        var message = exitCode == 0 ? null : $"The Rust backend exited with code {exitCode}.";
        FailPending(new InvalidOperationException(message ?? "The Rust backend exited."));
        if (!_stopping)
        {
            BackendExited?.Invoke(message);
        }
    }

    private void FailPending(Exception error)
    {
        foreach (var (requestId, completion) in _pending)
        {
            if (_pending.TryRemove(requestId, out _))
            {
                completion.TrySetException(error);
            }
        }
    }

    private static async Task SuppressFailure(Task task)
    {
        try
        {
            await task;
        }
        catch (Exception)
        {
            // Process shutdown races are expected while disposing the app.
        }
    }
}
