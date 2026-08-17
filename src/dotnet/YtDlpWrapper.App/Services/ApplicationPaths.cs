namespace YtDlpWrapper.Services;

public sealed record ApplicationPaths(string DataRoot)
{
    private const string DataDirectoryName = "YT-DLP Wrapper";

    public static ApplicationPaths Create()
    {
        var dataRoot = Path.Combine(GetPlatformDataDirectory(), DataDirectoryName);
        return new ApplicationPaths(dataRoot);
    }

    internal static string GetPlatformDataDirectory()
    {
        var folder = OperatingSystem.IsMacOS()
            ? Environment.SpecialFolder.ApplicationData
            : Environment.SpecialFolder.LocalApplicationData;
        var path = Environment.GetFolderPath(folder);
        if (string.IsNullOrWhiteSpace(path))
        {
            throw new InvalidOperationException("The operating system did not provide an application data directory.");
        }

        return path;
    }
}
