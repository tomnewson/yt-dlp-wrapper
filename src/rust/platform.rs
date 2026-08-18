use std::path::PathBuf;
use thiserror::Error;

pub const WINDOWS_X64: &str = "windows-x64";
pub const MACOS_ARM64: &str = "macos-arm64";

const WINDOWS_FFMPEG_RELEASE_API: &str =
    "https://api.github.com/repos/yt-dlp/FFmpeg-Builds/releases/latest";
const MACOS_FFMPEG_RELEASE_API: &str =
    "https://api.github.com/repos/shaka-project/static-ffmpeg-binaries/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolLayout {
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub deno: PathBuf,
}

impl ToolLayout {
    pub fn windows_x64() -> Self {
        Self {
            yt_dlp: "yt-dlp.exe".into(),
            ffmpeg: "ffmpeg.exe".into(),
            ffprobe: "ffprobe.exe".into(),
            deno: "deno.exe".into(),
        }
    }

    pub fn macos_arm64() -> Self {
        Self {
            yt_dlp: "yt-dlp".into(),
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            deno: "deno".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolPlatform {
    pub id: &'static str,
    pub yt_dlp_asset: &'static str,
    pub yt_dlp_checksums: &'static str,
    pub ffmpeg_release_api: &'static str,
    pub ffmpeg_asset: &'static str,
    pub ffprobe_asset: Option<&'static str>,
    pub ffmpeg_checksums: Option<&'static str>,
    pub deno_asset: &'static str,
    pub deno_checksums: &'static str,
    pub layout: ToolLayout,
}

impl ToolPlatform {
    pub fn current() -> Result<Self, PlatformError> {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Ok(Self::windows_x64())
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Ok(Self::macos_arm64())
        } else {
            Err(PlatformError::Unsupported(current_platform_id()))
        }
    }

    pub fn windows_x64() -> Self {
        Self {
            id: WINDOWS_X64,
            yt_dlp_asset: "yt-dlp.exe",
            yt_dlp_checksums: "SHA2-256SUMS",
            ffmpeg_release_api: WINDOWS_FFMPEG_RELEASE_API,
            ffmpeg_asset: "ffmpeg-master-latest-win64-gpl.zip",
            ffprobe_asset: None,
            ffmpeg_checksums: Some("checksums.sha256"),
            deno_asset: "deno-x86_64-pc-windows-msvc.zip",
            deno_checksums: "deno-x86_64-pc-windows-msvc.zip.sha256sum",
            layout: ToolLayout::windows_x64(),
        }
    }

    pub fn macos_arm64() -> Self {
        Self {
            id: MACOS_ARM64,
            yt_dlp_asset: "yt-dlp_macos",
            yt_dlp_checksums: "SHA2-256SUMS",
            ffmpeg_release_api: MACOS_FFMPEG_RELEASE_API,
            ffmpeg_asset: "ffmpeg-osx-arm64",
            ffprobe_asset: Some("ffprobe-osx-arm64"),
            ffmpeg_checksums: None,
            deno_asset: "deno-aarch64-apple-darwin.zip",
            deno_checksums: "deno-aarch64-apple-darwin.zip.sha256sum",
            layout: ToolLayout::macos_arm64(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlatformError {
    #[error("platform {0} is not supported by this release")]
    Unsupported(String),
}

pub fn current_platform_id() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn yt_dlp_filename_arguments(platform: &str) -> &'static [&'static str] {
    match platform {
        WINDOWS_X64 => &["--windows-filenames"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_layout_contains_relative_platform_paths() {
        let platform = ToolPlatform::windows_x64();
        assert_eq!(platform.id, WINDOWS_X64);
        assert!(platform.layout.yt_dlp.is_relative());
        assert_eq!(platform.layout.ffmpeg, PathBuf::from("ffmpeg.exe"));
        assert_eq!(
            yt_dlp_filename_arguments(platform.id),
            &["--windows-filenames"]
        );
    }

    #[test]
    fn macos_arm64_uses_native_standalone_tools() {
        let platform = ToolPlatform::macos_arm64();
        assert_eq!(platform.id, MACOS_ARM64);
        assert_eq!(platform.yt_dlp_asset, "yt-dlp_macos");
        assert_eq!(platform.ffprobe_asset, Some("ffprobe-osx-arm64"));
        assert_eq!(platform.layout.ffmpeg, PathBuf::from("ffmpeg"));
        assert_eq!(platform.layout.deno, PathBuf::from("deno"));
        assert!(yt_dlp_filename_arguments(platform.id).is_empty());
    }
}
