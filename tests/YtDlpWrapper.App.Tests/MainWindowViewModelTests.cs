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
        backend.Enqueue("initialize", InitializeResult());
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
        backend.Enqueue("initialize", InitializeResult());
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
        var platform = new FakePlatformServices();
        backend.Enqueue("startDownload", Json("""{"operationId":"operation-1"}"""));
        var viewModel = CreateViewModel(backend, platform);
        viewModel.Url = "https://example.com/video";
        viewModel.OutputFolder = "C:/Videos";
        viewModel.ToolsReady = true;
        viewModel.Busy = false;

        await viewModel.StartDownloadCommand.ExecuteAsync(null);
        backend.Raise(new BackendEvent(
            "operation-1",
            "operationCompleted",
            Json("""{"operationKind":"download","path":"C:/Videos/café.mp4"}""")));

        Assert.True(viewModel.Completed);
        Assert.True(viewModel.HasCompletedFile);
        Assert.Equal(100, viewModel.Progress);
        Assert.Equal("Saved C:/Videos/café.mp4", viewModel.StatusText);

        viewModel.OpenFolderCommand.Execute(null);
        Assert.Equal("C:/Videos/café.mp4", platform.RevealedPath);
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

        backend.Enqueue("initialize", InitializeResult());
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

    [Fact]
    public async Task ApplicationUpdateDownloadsStopsBackendAndApplies()
    {
        var backend = new FakeBackendClient();
        var updater = new FakeApplicationUpdater
        {
            NextUpdate = new ApplicationUpdate("0.2.0", new object()),
        };
        backend.Enqueue("initialize", InitializeResult());
        backend.Enqueue("checkTools", Json(
            """{"state":"ready","toolsReady":true,"canInstallTools":false,"updateSummary":"","statusText":"Ready."}"""));
        var viewModel = CreateViewModel(backend, updater: updater);

        await viewModel.InitializeAsync();

        Assert.True(viewModel.ShowApplicationUpdatePanel);
        Assert.Equal("0.2.0", viewModel.ApplicationUpdateVersion);

        await viewModel.InstallApplicationUpdateCommand.ExecuteAsync(null);

        Assert.True(updater.Downloaded);
        Assert.True(updater.Applied);
        Assert.Equal(1, backend.StopCount);
        Assert.Equal(100, viewModel.ApplicationUpdateProgress);
    }

    private static JsonElement Json(string value) => JsonDocument.Parse(value).RootElement.Clone();

    private static JsonElement InitializeResult() => Json(
        $$"""{"backendVersion":"{{ApplicationVersion.Current}}","outputFolder":"C:/Videos"}""");

    private static MainWindowViewModel CreateViewModel(
        FakeBackendClient backend,
        IPlatformServices? platform = null,
        IApplicationUpdater? updater = null) =>
        new(backend, platform ?? new FakePlatformServices(), action => action(), updater);
}

public sealed class PlatformServicesTests
{
    [Fact]
    public void BackendPipesUseBomlessUtf8()
    {
        var startInfo = new WindowsPlatformServices(() => null).CreateBackendStartInfo();

        Assert.Equal("utf-8", startInfo.StandardInputEncoding?.WebName);
        Assert.Equal("utf-8", startInfo.StandardOutputEncoding?.WebName);
        Assert.Equal("utf-8", startInfo.StandardErrorEncoding?.WebName);
        Assert.Empty(startInfo.StandardInputEncoding?.GetPreamble() ?? []);
    }

    [Fact]
    public void BackendUsesPersistentApplicationDataRoot()
    {
        var paths = new ApplicationPaths("C:/Users/test/AppData/Local/YT-DLP Wrapper");
        var startInfo = new WindowsPlatformServices(() => null, paths).CreateBackendStartInfo();

        Assert.Equal("--data-root", startInfo.ArgumentList[0]);
        Assert.Equal(paths.DataRoot, startInfo.ArgumentList[1]);
    }

}

internal sealed class FakeBackendClient : IBackendClient
{
    private readonly Dictionary<string, Queue<JsonElement>> _responses = [];

    public event Action<BackendEvent>? EventReceived;
    public event Action<string?>? BackendExited;
    public int StopCount { get; private set; }

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

    public Task StopAsync()
    {
        StopCount++;
        return Task.CompletedTask;
    }

    public ValueTask DisposeAsync() => ValueTask.CompletedTask;

    public void Raise(BackendEvent message) => EventReceived?.Invoke(message);
    public void Exit(string? message = null) => BackendExited?.Invoke(message);
}

internal sealed class FakeApplicationUpdater : IApplicationUpdater
{
    public ApplicationUpdate? NextUpdate { get; init; }
    public bool Downloaded { get; private set; }
    public bool Applied { get; private set; }
    public bool CanUpdate => true;
    public string CurrentVersion => ApplicationVersion.Current;

    public Task<ApplicationUpdate?> CheckForUpdatesAsync(CancellationToken cancellationToken = default) =>
        Task.FromResult(NextUpdate);

    public Task DownloadAsync(
        ApplicationUpdate update,
        Action<int>? progress = null,
        CancellationToken cancellationToken = default)
    {
        Downloaded = true;
        progress?.Invoke(100);
        return Task.CompletedTask;
    }

    public void ApplyAndRestart(ApplicationUpdate update) => Applied = true;
}

internal sealed class FakePlatformServices : IPlatformServices
{
    public string? RevealedPath { get; private set; }

    public ProcessStartInfo CreateBackendStartInfo() => new("backend");
    public Task<string?> PickOutputFolderAsync(string? currentFolder) => Task.FromResult(currentFolder);
    public void RevealFile(string path) => RevealedPath = path;
}
