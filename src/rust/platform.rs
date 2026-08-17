use std::path::PathBuf;
use thiserror::Error;

pub const WINDOWS_X64: &str = "windows-x64";

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
}

#[derive(Debug, Clone)]
pub struct ToolPlatform {
    pub id: &'static str,
    pub yt_dlp_asset: &'static str,
    pub yt_dlp_checksums: &'static str,
    pub ffmpeg_asset: &'static str,
    pub ffmpeg_checksums: &'static str,
    pub deno_asset: &'static str,
    pub deno_checksums: &'static str,
    pub layout: ToolLayout,
    pub use_windows_filenames: bool,
}

impl ToolPlatform {
    pub fn current() -> Result<Self, PlatformError> {
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Ok(Self::windows_x64())
        } else {
            Err(PlatformError::Unsupported(current_platform_id()))
        }
    }

    pub fn windows_x64() -> Self {
        Self {
            id: WINDOWS_X64,
            yt_dlp_asset: "yt-dlp.exe",
            yt_dlp_checksums: "SHA2-256SUMS",
            ffmpeg_asset: "ffmpeg-master-latest-win64-gpl.zip",
            ffmpeg_checksums: "checksums.sha256",
            deno_asset: "deno-x86_64-pc-windows-msvc.zip",
            deno_checksums: "deno-x86_64-pc-windows-msvc.zip.sha256sum",
            layout: ToolLayout::windows_x64(),
            use_windows_filenames: true,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_layout_contains_relative_platform_paths() {
        let platform = ToolPlatform::windows_x64();
        assert_eq!(platform.id, WINDOWS_X64);
        assert!(platform.layout.yt_dlp.is_relative());
        assert_eq!(platform.layout.ffmpeg, PathBuf::from("ffmpeg.exe"));
        assert!(platform.use_windows_filenames);
    }
}
