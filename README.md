# yt-dlp-wrapper

`yt-dlp-wrapper` is a small Windows GUI for downloading one video at a time. It uses the current yt-dlp nightly build and produces editor-friendly files:

- Video: MP4 with H.264 video and AAC audio
- Audio only: M4A with AAC audio

Before downloading video, the application checks the available formats. The quality slider can
limit the video to 1080p or 1440p, or retain the default Best setting. A limited setting selects
the highest available resolution at or below that limit; Best selects the maximum advertised
resolution. At the selected resolution the application prefers H.264 and AAC streams so they can
be used directly or remuxed without generation loss. It downloads another video codec and
transcodes to H.264 only when H.264 is unavailable at that resolution. Audio is converted
separately only when no AAC stream is available.

When video conversion is required, the application tests NVIDIA NVENC, Intel Quick Sync, and AMD
AMF in that order by encoding a real test frame. It uses the first working GPU encoder and
automatically retries with CPU-based x264 if GPU encoding is unavailable or the hardware encode
fails. Remuxing and already-compatible downloads do not invoke a video encoder.

For video conversion, FFmpeg reads the source dimensions and average frame rate and applies the
[YouTube SDR encoding guidance](https://support.google.com/youtube/answer/1722171). GPU encoders use
the target and maximum; the CRF-based CPU fallback uses the same maximum. High frame rate means 48
fps or greater. Values are target/maximum Mbps:

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

The application is designed for Windows 10 and Windows 11 on x64 computers. Linux and macOS packages are not part of the MVP.

The release executable statically links the Microsoft C runtime. A separate Visual C++ Redistributable installation is not required for the Rust application.

## Use

1. Extract the release ZIP to a writable folder.
2. Start `yt-dlp-wrapper.exe`.
3. Approve the first tool download. The application downloads yt-dlp, FFmpeg, ffprobe, and Deno beside the application.
4. Paste one HTTP or HTTPS video URL.
5. Select a video quality, or select audio-only output.
6. Select an output folder and start the download.

The first setup needs an internet connection and several hundred MiB of free space. Later runs can use the cached tools without an internet connection. The complete application folder can be moved after the application is closed.

Do not place the application in `Program Files` or another read-only folder. The application stores its tools and settings in `yt-dlp-wrapper-data` beside the executable.

## Updates

The application checks these official release sources at startup:

- yt-dlp nightly builds
- FFmpeg builds for yt-dlp
- Deno stable releases

It verifies each downloaded archive or executable with the SHA-256 value published by that project. It activates a new toolset only after all tools pass a startup test. A failed update does not replace the cached toolset.

## Build

Install the stable Rust toolchain and the MSVC Windows build tools. Then run:

```powershell
cargo test
cargo build --release
```

Enable the repository's pre-commit checks in each new checkout:

```powershell
git config core.hooksPath .githooks
```

The hook checks formatting and compilation, runs the tests, and treats every Clippy warning as an
error before allowing a commit.

The executable is written to `target\release\yt-dlp-wrapper.exe`.

## Privacy and security

- The application passes URLs directly to yt-dlp without a command shell.
- It does not write the submitted URL to its normal log file.
- It ignores external yt-dlp configuration files.
- It does not support cookies, accounts, playlists, or private-video login in the MVP.

Only download media when you have permission to do so. The user is responsible for applicable terms and copyright law.

## Licence

This program is free software under GNU GPL version 3 only. See `COPYING`.

The application uses Slint under its GPLv3 option. Downloaded tools have their own licences. Available third-party licence files are stored in each managed tool directory.
