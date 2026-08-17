use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::platform::{ToolLayout, WINDOWS_X64};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    Video,
    AudioOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    P1080,
    P1440,
    Best,
}

impl VideoQuality {
    pub fn from_slider_value(value: f32) -> Self {
        match value.round() as i32 {
            i32::MIN..=0 => Self::P1080,
            1 => Self::P1440,
            _ => Self::Best,
        }
    }

    pub fn maximum_dimension(self) -> Option<u32> {
        match self {
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
            Self::Best => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    pub mode: DownloadMode,
    pub video_quality: VideoQuality,
    pub output_directory: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JobPhase {
    Preparing,
    Downloading,
    Inspecting,
    Remuxing,
    Converting,
    Finalizing,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub phase: JobPhase,
    pub fraction: Option<f32>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_second: Option<u64>,
    pub message: String,
}

impl ProgressUpdate {
    pub fn message(phase: JobPhase, message: impl Into<String>) -> Self {
        Self {
            phase,
            fraction: None,
            downloaded_bytes: None,
            total_bytes: None,
            speed_bytes_per_second: None,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveToolset {
    pub id: String,
    pub yt_dlp_version: String,
    pub ffmpeg_version: String,
    pub deno_version: String,
    pub directory: PathBuf,
    #[serde(default = "legacy_platform")]
    pub platform: String,
    #[serde(default = "legacy_yt_dlp_path")]
    pub yt_dlp_path: PathBuf,
    #[serde(default = "legacy_ffmpeg_path")]
    pub ffmpeg_path: PathBuf,
    #[serde(default = "legacy_ffprobe_path")]
    pub ffprobe_path: PathBuf,
    #[serde(default = "legacy_deno_path")]
    pub deno_path: PathBuf,
}

impl ActiveToolset {
    pub fn yt_dlp(&self) -> PathBuf {
        self.directory.join(&self.yt_dlp_path)
    }

    pub fn ffmpeg(&self) -> PathBuf {
        self.directory.join(&self.ffmpeg_path)
    }

    pub fn ffprobe(&self) -> PathBuf {
        self.directory.join(&self.ffprobe_path)
    }

    pub fn deno(&self) -> PathBuf {
        self.directory.join(&self.deno_path)
    }
}

fn legacy_platform() -> String {
    WINDOWS_X64.into()
}

fn legacy_yt_dlp_path() -> PathBuf {
    ToolLayout::windows_x64().yt_dlp
}

fn legacy_ffmpeg_path() -> PathBuf {
    ToolLayout::windows_x64().ffmpeg
}

fn legacy_ffprobe_path() -> PathBuf {
    ToolLayout::windows_x64().ffprobe
}

fn legacy_deno_path() -> PathBuf {
    ToolLayout::windows_x64().deno
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_toolset_json_uses_windows_layout() {
        let active: ActiveToolset = serde_json::from_str(
            r#"{"id":"one","yt_dlp_version":"1","ffmpeg_version":"2","deno_version":"3","directory":"tools/one"}"#,
        )
        .unwrap();
        assert_eq!(active.platform, WINDOWS_X64);
        assert_eq!(active.yt_dlp(), PathBuf::from("tools/one/yt-dlp.exe"));
    }
}
