using Velopack;
using Velopack.Sources;

namespace YtDlpWrapper.Services;

public static class ApplicationUpdater
{
    private const string RepositoryUrl = "https://github.com/tomnewson/yt-dlp-wrapper";

    public static IApplicationUpdater Create()
    {
        try
        {
            return new VelopackApplicationUpdater(new UpdateManager(
                new GithubSource(RepositoryUrl, accessToken: null, prerelease: false)));
        }
        catch (Exception)
        {
            return UnavailableApplicationUpdater.Instance;
        }
    }
}

internal sealed class VelopackApplicationUpdater(UpdateManager manager) : IApplicationUpdater
{
    public bool CanUpdate => manager.IsInstalled;

    public string CurrentVersion =>
        manager.CurrentVersion?.ToString() ?? ApplicationVersion.Current;

    public async Task<ApplicationUpdate?> CheckForUpdatesAsync(
        CancellationToken cancellationToken = default)
    {
        if (!CanUpdate)
        {
            return null;
        }

        cancellationToken.ThrowIfCancellationRequested();
        var update = await manager.CheckForUpdatesAsync();
        cancellationToken.ThrowIfCancellationRequested();
        return update is null
            ? null
            : new ApplicationUpdate(update.TargetFullRelease.Version.ToString(), update);
    }

    public Task DownloadAsync(
        ApplicationUpdate update,
        Action<int>? progress = null,
        CancellationToken cancellationToken = default) =>
        manager.DownloadUpdatesAsync(GetNativeUpdate(update), progress, cancellationToken);

    public void ApplyAndRestart(ApplicationUpdate update) =>
        manager.ApplyUpdatesAndRestart(GetNativeUpdate(update));

    private static UpdateInfo GetNativeUpdate(ApplicationUpdate update) =>
        update.NativeUpdate as UpdateInfo ??
        throw new ArgumentException("The application update did not originate from Velopack.", nameof(update));
}

internal sealed class UnavailableApplicationUpdater : IApplicationUpdater
{
    public static UnavailableApplicationUpdater Instance { get; } = new();

    public bool CanUpdate => false;
    public string CurrentVersion => ApplicationVersion.Current;

    public Task<ApplicationUpdate?> CheckForUpdatesAsync(CancellationToken cancellationToken = default) =>
        Task.FromResult<ApplicationUpdate?>(null);

    public Task DownloadAsync(
        ApplicationUpdate update,
        Action<int>? progress = null,
        CancellationToken cancellationToken = default) =>
        Task.FromException(new InvalidOperationException("Application updates are unavailable in this build."));

    public void ApplyAndRestart(ApplicationUpdate update) =>
        throw new InvalidOperationException("Application updates are unavailable in this build.");
}
