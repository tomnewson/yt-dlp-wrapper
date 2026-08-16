# yt-dlp-wrapper

`yt-dlp-wrapper` is a small Windows GUI for downloading one video at a time. It uses the current yt-dlp nightly build and produces editor-friendly files:

- Video: MP4 with H.264 video and AAC audio
- Audio only: M4A with AAC audio

The application is designed for Windows 10 and Windows 11 on x64 computers. Linux and macOS packages are not part of the MVP.

The release executable statically links the Microsoft C runtime. A separate Visual C++ Redistributable installation is not required for the Rust application.

## Use

1. Extract the release ZIP to a writable folder.
2. Start `yt-dlp-wrapper.exe`.
3. Approve the first tool download. The application downloads yt-dlp, FFmpeg, ffprobe, and Deno beside the application.
4. Paste one HTTP or HTTPS video URL.
5. Select video or audio-only output.
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
