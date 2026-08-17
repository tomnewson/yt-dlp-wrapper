using System.Diagnostics;

namespace YtDlpWrapper.Services;

public interface IPlatformServices
{
    string PlatformId { get; }
    string DataRoot { get; }
    ProcessStartInfo CreateBackendStartInfo();
    Task<string?> PickOutputFolderAsync(string? currentFolder);
    void RevealFile(string path);
}
