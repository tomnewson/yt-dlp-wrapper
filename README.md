# yt-dlp-wrapper

`yt-dlp-wrapper` is a small Windows and Apple Silicon macOS GUI for downloading one video at a time. The interface is built with C# and Avalonia; a private Rust sidecar performs tool management and media processing. It uses the current yt-dlp nightly build and produces editor-friendly files:

- Video: MP4 with H.264 video and AAC audio
- Audio only: M4A with AAC audio

Before downloading video, the application checks the available formats. The quality slider can limit the video to 1080p or 1440p, or retain the default Best setting. A limited setting selects the highest available resolution at or below that limit; Best selects the maximum advertised resolution. At the selected resolution, the application prefers H.264 and AAC streams so they can be used directly or remuxed without generation loss. It downloads another video codec and transcodes to H.264 only when H.264 is unavailable at that resolution. Audio is converted separately only when no AAC stream is available.

When video conversion is required, the Windows backend tests NVIDIA NVENC, Intel Quick Sync, and AMD AMF in that order by encoding a real test frame. On Apple Silicon macOS it tests Apple VideoToolbox. It uses the first working GPU encoder and automatically retries with CPU-based x264 if GPU encoding is unavailable or the hardware encode fails. Remuxing and already-compatible downloads do not invoke a video encoder.

For video conversion, FFmpeg reads the source dimensions and average frame rate and applies the [YouTube SDR encoding guidance](https://support.google.com/youtube/answer/1722171). GPU encoders use the target and maximum; the CRF-based CPU fallback uses the same maximum. High frame rate means 48 fps or greater. Values are target/maximum Mbps:

| Resolution | Standard frame rate | High frame rate |
| --- | ---: | ---: |
| 8K | 80/160 | 120/240 |
| 4K | 35/45 | 53/68 |
| 1440p | 16/16 | 24/24 |
| 1080p | 8/8 | 12/12 |
| 720p | 5/5 | 7.5/7.5 |
| 480p | 2.5/2.5 | 4/4 |
| 360p or lower | 1/1 | 1.5/1.5 |

Portrait video is classified by its shorter dimension, so 1080x1920 uses the 1080p tier.

The application supports Windows 10 and Windows 11 on x64 computers and macOS 14 or newer on Apple Silicon (arm64).

## Use

1. On Windows, download `YT-DLP-Wrapper-Installer.exe` from the latest release and run it. On Apple Silicon macOS, download `YT-DLP-Wrapper-macOS-arm64-Installer.pkg` and run it. Portable ZIPs are also provided for both platforms.
2. Start YT-DLP Wrapper. Portable builds must keep the Rust backend beside the frontend; the supplied packages already have the correct layout.
3. Approve the first tool download. The application downloads yt-dlp, FFmpeg, ffprobe, and Deno into its application data folder.
4. Paste one HTTP or HTTPS video URL.
5. Select a video quality, or select audio-only output.
6. Select an output folder and start the download.

The first setup needs an internet connection and several hundred MiB of free space. Later runs can use cached tools without an internet connection.

Tools, settings, and logs are stored independently of the application files: `%LOCALAPPDATA%\YT-DLP Wrapper` on Windows and `~/Library/Application Support/YT-DLP Wrapper` on macOS. Updating or replacing the application therefore preserves the tool cache and configuration.

## Architecture

The Avalonia process owns the window and operating-system integration. It starts the Rust backend without a console window and exchanges versioned, newline-delimited JSON over redirected standard input and output. Media bytes never cross this protocol. Rust owns tool updates, validation, downloads, transcoding, cancellation, configuration, and logs.

The sidecar protocol is private to matching application releases. Standard output is reserved for protocol messages, and submitted video URLs are not written to normal application logs.

## Updates

Installed builds check this repository's GitHub Releases at startup. When an application update is available, the interface can download it and restart into the new version. Release packages are produced with Velopack; delta packages are used when a compatible previous release is available. Development and portable builds do not attempt to update themselves.

Application code and managed tools have separate update lifecycles. Replacing the application does not replace the managed tools or settings.

The backend checks these official release sources at startup:

- yt-dlp nightly builds
- FFmpeg builds for yt-dlp on Windows; Shaka Project static FFmpeg builds on macOS arm64
- Deno stable releases

It verifies each downloaded archive or executable with the SHA-256 value published by that project. It activates a new platform-specific toolset only after all tools pass a startup test. A failed update does not replace the cached toolset.

## Build

Install the stable Rust toolchain and .NET 10 SDK. Windows builds additionally need the MSVC build tools. Restore and test both parts:

```powershell
dotnet restore YtDlpWrapper.slnx
cargo test --all-targets --locked
dotnet test YtDlpWrapper.slnx --no-restore -m:1 --disable-build-servers
```

Build both development executables, place the Rust sidecar beside the frontend, and start the app:

```powershell
./scripts/run-dev.ps1
```

On an Apple Silicon Mac, use the native development script instead:

```bash
./scripts/run-dev.sh
```

Build the Windows backend and publish the self-contained Avalonia frontend:

```powershell
$buildVersion = (git describe --tags --abbrev=0 --match "v[0-9]*").Substring(1)
$env:YT_DLP_WRAPPER_VERSION = $buildVersion
cargo build --release --target x86_64-pc-windows-msvc --locked
dotnet publish src/dotnet/YtDlpWrapper.App/YtDlpWrapper.App.csproj `
  --configuration Release --runtime win-x64 --self-contained true `
  -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true `
  -p:PublishTrimmed=false --output dist/yt-dlp-wrapper
Copy-Item target/x86_64-pc-windows-msvc/release/yt-dlp-wrapper-backend.exe dist/yt-dlp-wrapper/
```

Build an ad-hoc-signed, self-contained macOS arm64 app bundle and portable ZIP on an Apple Silicon Mac:

```bash
./scripts/build-macos-arm64.sh
open 'dist/macos-arm64/YT-DLP Wrapper.app'
```

The resulting frontend and Rust backend are native arm64 Mach-O executables. The ZIP preserves the standard `.app` bundle layout and executable permissions. Distribution outside local development still requires an Apple Developer ID signature and notarization to avoid Gatekeeper warnings.

To exercise a clean first-run download, checksum verification, and tool startup without changing your real application data, run:

```bash
./scripts/tool-install-smoke.sh \
  'dist/macos-arm64/YT-DLP Wrapper.app/Contents/MacOS/yt-dlp-wrapper-backend'
```

Git tags are the application version source: builds use the most recent reachable `v*` tag, and tag `v0.1.2` produces frontend and backend binaries at version `0.1.2`. Pushing a version tag runs coordinated Windows and Apple Silicon macOS builds. The release is published only after both succeed, with an installer, portable package, full update package, optional delta, and platform update feed for each operating system.

Create a release by tagging the commit to publish and pushing the tag:

```bash
git tag v0.1.4
git push origin v0.1.4
```

The tag must be a semantic version beginning with `v`. GitHub Actions creates or updates the matching GitHub Release and adds the user-facing downloads plus `releases.win.json` and `releases.osx.json` for automatic updates. The first macOS Velopack release is a full update baseline; later releases can generate delta packages from it.

The macOS packages are currently ad-hoc signed rather than Developer ID signed and notarized. A first-time user may need to approve the app or installer in System Settings under Privacy & Security. Once installed, the Velopack-enabled app can replace itself during subsequent updates; its tools, settings, and logs remain in `~/Library/Application Support/YT-DLP Wrapper`.

Enable the repository's pre-commit checks in each new checkout:

```powershell
git config core.hooksPath .githooks
```

The hook checks Rust and C# formatting, compiles both projects, runs both test suites, and treats every Clippy warning as an error.

## Privacy and security

- The backend passes URLs directly to yt-dlp without a command shell.
- It does not write submitted URLs to its normal log file.
- It ignores external yt-dlp configuration files.
- It does not support cookies, accounts, playlists, or private-video login in the MVP.
- Avalonia starts only the backend shipped beside the frontend and terminates its owned process during shutdown.

Only download media when you have permission to do so. The user is responsible for applicable terms and copyright law.

## Licence

This program is free software under GNU GPL version 3 only. See `COPYING`.

Avalonia, .NET, the .NET Community Toolkit, and Velopack use their respective permissive licences. Downloaded tools have their own licences. Available third-party licence files are stored in each managed tool directory; see `THIRD_PARTY.md` for the project-level summary.
