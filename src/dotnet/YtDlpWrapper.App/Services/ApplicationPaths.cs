namespace YtDlpWrapper.Services;

public sealed record ApplicationPaths(string DataRoot)
{
    private const string DataDirectoryName = "YT-DLP Wrapper";
    private const string DataRootEnvironmentVariable = "YT_DLP_WRAPPER_DATA_ROOT";

    public static ApplicationPaths Create()
    {
        var overrideRoot = Environment.GetEnvironmentVariable(DataRootEnvironmentVariable);
        if (!string.IsNullOrWhiteSpace(overrideRoot))
        {
            return new ApplicationPaths(Path.GetFullPath(overrideRoot));
        }

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
