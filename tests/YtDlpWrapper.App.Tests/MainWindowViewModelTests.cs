using System.Diagnostics;
using System.Text.Json;
using Xunit;
using YtDlpWrapper.Services;
using YtDlpWrapper.ViewModels;

namespace YtDlpWrapper.App.Tests;

public sealed class MainWindowViewModelTests
{
    [Fact]
    public async Task InitializeLoadsFolderAndReadyState()
    {
        var backend = new FakeBackendClient();
        backend.Enqueue("initialize", Json("""{"backendVersion":"0.1.0","outputFolder":"C:/Videos"}"""));
        backend.Enqueue("checkTools", Json(
            """{"state":"ready","toolsReady":true,"canInstallTools":false,"updateSummary":"","statusText":"Ready. All tools are current."}"""));
        var viewModel = CreateViewModel(backend);

        await viewModel.InitializeAsync();

        Assert.Equal("C:/Videos", viewModel.OutputFolder);
        Assert.True(viewModel.ToolsReady);
        Assert.False(viewModel.Busy);
        Assert.Equal("Ready. All tools are current.", viewModel.StatusText);
    }

    [Fact]
    public async Task UpdateStateRequiresDeferralBeforeDownload()
    {
        var backend = new FakeBackendClient();
        backend.Enqueue("initialize", Json("""{"backendVersion":"0.1.0","outputFolder":"C:/Videos"}"""));
        backend.Enqueue("checkTools", Json(
            """{"state":"updateAvailable","toolsReady":true,"canInstallTools":true,"updateSummary":"yt-dlp 1","statusText":"Updates are available."}"""));
        var viewModel = CreateViewModel(backend);
        viewModel.Url = "https://example.com/video";

        await viewModel.InitializeAsync();
        Assert.False(viewModel.CanDownload);

        viewModel.DeferUpdateCommand.Execute(null);
        Assert.True(viewModel.CanDownload);
        Assert.Equal("Ready. The cached tools will be used.", viewModel.StatusText);
    }

    [Fact]
    public async Task DownloadCompletionMakesFileRevealAvailable()
    {
        var backend = new FakeBackendClient();
        backend.Enqueue("startDownload", Json("""{"operationId":"operation-1"}"""));
        var viewModel = CreateViewModel(backend);
        viewModel.Url = "https://example.com/video";
        viewModel.OutputFolder = "C:/Videos";
        viewModel.ToolsReady = true;
        viewModel.Busy = false;

        await viewModel.StartDownloadCommand.ExecuteAsync(null);
        backend.Raise(new BackendEvent(
            "operation-1",
            "operationCompleted",
            Json("""{"operationKind":"download","path":"C:/Videos/file.mp4"}""")));

        Assert.True(viewModel.Completed);
        Assert.True(viewModel.HasCompletedFile);
        Assert.Equal(100, viewModel.Progress);
    }

    [Fact]
    public async Task CancelledUpdateKeepsPreviouslyCachedToolsReady()
    {
        var backend = new FakeBackendClient();
        backend.Enqueue("installTools", Json("""{"operationId":"operation-2"}"""));
        var viewModel = CreateViewModel(backend);
        viewModel.ToolsReady = true;
        viewModel.UpdateAvailable = true;
        viewModel.CanInstallTools = true;
        viewModel.Busy = false;

        await viewModel.InstallToolsCommand.ExecuteAsync(null);
        backend.Raise(new BackendEvent(
            "operation-2",
            "operationCancelled",
            Json("""{"operationKind":"toolInstall"}""")));

        Assert.True(viewModel.ToolsReady);
        Assert.False(viewModel.SetupRequired);
        Assert.True(viewModel.CanInstallTools);
    }

    [Fact]
    public async Task FailedStartupCanBeRetriedExplicitly()
    {
        var backend = new FakeBackendClient();
        var viewModel = CreateViewModel(backend);

        await viewModel.InitializeAsync();
        Assert.True(viewModel.ShowRestartButton);

        backend.Enqueue("initialize", Json("""{"backendVersion":"0.1.0","outputFolder":"C:/Videos"}"""));
        backend.Enqueue("checkTools", Json(
            """{"state":"ready","toolsReady":true,"canInstallTools":false,"updateSummary":"","statusText":"Ready."}"""));
        await viewModel.RestartBackendCommand.ExecuteAsync(null);

        Assert.False(viewModel.EngineUnavailable);
        Assert.True(viewModel.ToolsReady);
    }

    [Fact]
    public void BackendExitDisablesOperations()
    {
        var backend = new FakeBackendClient();
        var viewModel = CreateViewModel(backend);
        viewModel.ToolsReady = true;
        viewModel.Busy = false;

        backend.Exit("Backend failed.");

        Assert.True(viewModel.EngineUnavailable);
        Assert.False(viewModel.ToolsReady);
        Assert.False(viewModel.CanDownload);
        Assert.Equal("Backend failed.", viewModel.DetailsText);
    }

    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();

    private static MainWindowViewModel CreateViewModel(FakeBackendClient backend) =>
        new(backend, new FakePlatformServices(), action => action());
}

internal sealed class FakeBackendClient : IBackendClient
{
    private readonly Dictionary<string, Queue<JsonElement>> _responses = [];

    public event Action<BackendEvent>? EventReceived;
    public event Action<string?>? BackendExited;

    public void Enqueue(string method, JsonElement response)
    {
        if (!_responses.TryGetValue(method, out var values))
        {
            values = new Queue<JsonElement>();
            _responses[method] = values;
        }
        values.Enqueue(response);
    }

    public Task StartAsync(CancellationToken cancellationToken = default) => Task.CompletedTask;

    public Task<JsonElement> SendAsync(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default) =>
        Task.FromResult(_responses[method].Dequeue());

    public Task StopAsync() => Task.CompletedTask;

    public ValueTask DisposeAsync() => ValueTask.CompletedTask;

    public void Raise(BackendEvent message) => EventReceived?.Invoke(message);
    public void Exit(string? message = null) => BackendExited?.Invoke(message);
}

internal sealed class FakePlatformServices : IPlatformServices
{
    public ProcessStartInfo CreateBackendStartInfo() => new("backend");
    public Task<string?> PickOutputFolderAsync(string? currentFolder) => Task.FromResult(currentFolder);
    public void RevealFile(string path) { }
}
