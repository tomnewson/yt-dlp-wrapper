use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadMode {
    Video,
    AudioOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    P1080,
    P1440,
    P2160,
    Best,
}

impl VideoQuality {
    pub fn from_slider_value(value: f32) -> Self {
        match value.round() as i32 {
            i32::MIN..=0 => Self::P1080,
            1 => Self::P1440,
            2 => Self::P2160,
            _ => Self::Best,
        }
    }

    pub fn maximum_dimension(self) -> Option<u32> {
        match self {
            Self::P1080 => Some(1080),
            Self::P1440 => Some(1440),
            Self::P2160 => Some(2160),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl ActiveToolset {
    pub fn yt_dlp(&self) -> PathBuf {
        self.directory.join("yt-dlp.exe")
    }

    pub fn ffmpeg(&self) -> PathBuf {
        self.directory.join("ffmpeg.exe")
    }

    pub fn ffprobe(&self) -> PathBuf {
        self.directory.join("ffprobe.exe")
    }

    pub fn deno(&self) -> PathBuf {
        self.directory.join("deno.exe")
    }
}
