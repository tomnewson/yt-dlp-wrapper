using System.Diagnostics;
using Avalonia.Controls;
using Avalonia.Platform.Storage;

namespace YtDlpWrapper.Services;

public abstract class PlatformServices(Func<Window?> getWindow) : IPlatformServices
{
    protected Func<Window?> GetWindow { get; } = getWindow;

    public abstract string PlatformId { get; }
    public abstract string DataRoot { get; }
    protected abstract string BackendFileName { get; }

    public static IPlatformServices Create(Func<Window?> getWindow)
    {
        if (OperatingSystem.IsWindows())
        {
            return new WindowsPlatformServices(getWindow);
        }

        if (OperatingSystem.IsMacOS())
        {
            return new MacOsPlatformServices(getWindow);
        }

        return new UnsupportedPlatformServices(getWindow);
    }

    public virtual ProcessStartInfo CreateBackendStartInfo()
    {
        var info = new ProcessStartInfo(Path.Combine(AppContext.BaseDirectory, BackendFileName))
        {
            UseShellExecute = false,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
        };
        info.ArgumentList.Add("--data-root");
        info.ArgumentList.Add(DataRoot);
        return info;
    }

    public async Task<string?> PickOutputFolderAsync(string? currentFolder)
    {
        var storage = GetWindow()?.StorageProvider;
        if (storage is null)
        {
            return null;
        }

        IStorageFolder? start = null;
        if (!string.IsNullOrWhiteSpace(currentFolder))
        {
            try
            {
                start = await storage.TryGetFolderFromPathAsync(new Uri(Path.GetFullPath(currentFolder)));
            }
            catch (Exception)
            {
                // A stale saved folder should not prevent opening the picker.
            }
        }

        var selected = await storage.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = "Choose output folder",
            AllowMultiple = false,
            SuggestedStartLocation = start,
        });
        return selected.Count == 0 ? null : selected[0].TryGetLocalPath();
    }

    public abstract void RevealFile(string path);

    protected static void StartDetached(string executable, params string[] arguments)
    {
        var info = new ProcessStartInfo(executable) { UseShellExecute = false };
        foreach (var argument in arguments)
        {
            info.ArgumentList.Add(argument);
        }

        Process.Start(info);
    }
}

internal sealed class WindowsPlatformServices(Func<Window?> getWindow) : PlatformServices(getWindow)
{
    public override string PlatformId => "windows-x64";
    public override string DataRoot => Path.Combine(AppContext.BaseDirectory, "yt-dlp-wrapper-data");
    protected override string BackendFileName => "yt-dlp-wrapper-backend.exe";

    public override void RevealFile(string path) => StartDetached("explorer.exe", "/select,", path);
}

internal sealed class MacOsPlatformServices(Func<Window?> getWindow) : PlatformServices(getWindow)
{
    public override string PlatformId => "macos-arm64";
    public override string DataRoot => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "YT-DLP Wrapper");
    protected override string BackendFileName => "yt-dlp-wrapper-backend";

    public override void RevealFile(string path) => StartDetached("/usr/bin/open", "-R", path);
}

internal sealed class UnsupportedPlatformServices(Func<Window?> getWindow) : PlatformServices(getWindow)
{
    public override string PlatformId => $"{Environment.OSVersion.Platform}-{System.Runtime.InteropServices.RuntimeInformation.ProcessArchitecture}";
    public override string DataRoot => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "yt-dlp-wrapper");
    protected override string BackendFileName => "yt-dlp-wrapper-backend";

    public override void RevealFile(string path) => throw new PlatformNotSupportedException();
}
