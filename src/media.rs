use crate::model::{ActiveToolset, DownloadMode, DownloadRequest, JobPhase, ProgressUpdate};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
};
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PROGRESS_PREFIX: &str = "__YTDLP_WRAPPER_PROGRESS__";
const FILE_PREFIX: &str = "__YTDLP_WRAPPER_FILE__";

pub type MediaProgress = Arc<dyn Fn(ProgressUpdate) + Send + Sync>;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("media information is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("yt-dlp failed: {0}")]
    YtDlp(String),
    #[error("FFmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("yt-dlp did not report the downloaded file")]
    MissingOutput,
    #[error("the downloaded file has no supported media stream")]
    MissingStream,
}

#[derive(Debug, Deserialize)]
struct ProbeResult {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    pix_fmt: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

pub async fn download_media(
    tools: ActiveToolset,
    request: DownloadRequest,
    cancel: CancellationToken,
    progress: MediaProgress,
) -> Result<PathBuf, MediaError> {
    (progress)(ProgressUpdate::message(
        JobPhase::Preparing,
        "Preparing download…",
    ));
    fs::create_dir_all(&request.output_directory).await?;
    let staging = request
        .output_directory
        .join(format!(".yt-dlp-wrapper-{}", Uuid::new_v4()));
    fs::create_dir_all(&staging).await?;

    let result = run_pipeline(&tools, &request, &staging, &cancel, &progress).await;
    let _ = fs::remove_dir_all(&staging).await;
    match &result {
        Err(MediaError::Cancelled) => (progress)(ProgressUpdate::message(
            JobPhase::Cancelled,
            "Download cancelled.",
        )),
        Err(_) => (progress)(ProgressUpdate::message(
            JobPhase::Failed,
            "The download failed.",
        )),
        Ok(_) => {}
    }
    result
}

async fn run_pipeline(
    tools: &ActiveToolset,
    request: &DownloadRequest,
    staging: &Path,
    cancel: &CancellationToken,
    progress: &MediaProgress,
) -> Result<PathBuf, MediaError> {
    let downloaded = run_yt_dlp(tools, request, staging, cancel, progress).await?;
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }

    (progress)(ProgressUpdate::message(
        JobPhase::Inspecting,
        "Checking media codecs…",
    ));
    let probe = probe_media(&tools.ffprobe(), &downloaded, cancel).await?;
    let output = unique_output_path(
        &request.output_directory,
        downloaded
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("download"),
        match request.mode {
            DownloadMode::Video => "mp4",
            DownloadMode::AudioOnly => "m4a",
        },
    );

    convert_media(
        &tools.ffmpeg(),
        &downloaded,
        &output,
        request.mode,
        &probe,
        cancel,
        progress,
    )
    .await?;

    (progress)(ProgressUpdate {
        phase: JobPhase::Completed,
        fraction: Some(1.0),
        downloaded_bytes: None,
        total_bytes: None,
        speed_bytes_per_second: None,
        eta_seconds: None,
        message: format!("Saved {}", output.display()),
    });
    Ok(output)
}

async fn run_yt_dlp(
    tools: &ActiveToolset,
    request: &DownloadRequest,
    staging: &Path,
    cancel: &CancellationToken,
    progress: &MediaProgress,
) -> Result<PathBuf, MediaError> {
    let final_path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let diagnostics = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(30)));
    let mut command = Command::new(tools.yt_dlp());
    command
        .arg("--no-config")
        .arg("--no-update")
        .arg("--no-playlist")
        .arg("--windows-filenames")
        .arg("--newline")
        .arg("--ffmpeg-location")
        .arg(&tools.directory)
        .arg("--js-runtimes")
        .arg(format!("deno:{}", tools.deno().display()))
        .arg("--paths")
        .arg(format!("home:{}", staging.display()))
        .arg("--output")
        .arg("%(title).180B [%(id)s].%(ext)s")
        .arg("--progress-template")
        .arg(format!(
            "download:{PROGRESS_PREFIX}%(progress._percent_str)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.speed)s|%(progress.eta)s"
        ))
        .arg("--print")
        .arg(format!("after_move:{FILE_PREFIX}%(filepath)j"));

    match request.mode {
        DownloadMode::Video => {
            command.args(["--format", "bv*+ba/b", "--merge-output-format", "mkv"]);
        }
        DownloadMode::AudioOnly => {
            command.args(["--format", "ba/b"]);
        }
    }
    command.arg("--").arg(&request.url);
    configure_child(&mut command, &tools.directory);

    let output_path = Arc::clone(&final_path);
    let progress_sink = Arc::clone(progress);
    let stdout_handler = Arc::new(move |line: String| {
        if let Some(value) = line.strip_prefix(PROGRESS_PREFIX) {
            if let Some(update) = parse_ytdlp_progress(value) {
                (progress_sink)(update);
            }
        } else if let Some(value) = line.strip_prefix(FILE_PREFIX)
            && let Ok(path) = serde_json::from_str::<String>(value)
        {
            *output_path.lock().expect("output path mutex poisoned") = Some(PathBuf::from(path));
        }
    });
    let redacted_url = request.url.clone();
    let diagnostic_sink = Arc::clone(&diagnostics);
    let stderr_handler = Arc::new(move |line: String| {
        let safe = line.replace(&redacted_url, "[URL]");
        let mut lines = diagnostic_sink.lock().expect("diagnostics mutex poisoned");
        if lines.len() == 30 {
            lines.pop_front();
        }
        lines.push_back(safe);
    });

    let status = run_command(command, cancel, stdout_handler, stderr_handler).await?;
    if !status.success() {
        let details = diagnostics
            .lock()
            .expect("diagnostics mutex poisoned")
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(MediaError::YtDlp(if details.is_empty() {
            status.to_string()
        } else {
            details
        }));
    }
    final_path
        .lock()
        .expect("output path mutex poisoned")
        .clone()
        .ok_or(MediaError::MissingOutput)
}

async fn probe_media(
    ffprobe: &Path,
    input: &Path,
    cancel: &CancellationToken,
) -> Result<ProbeResult, MediaError> {
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let mut command = Command::new(ffprobe);
    command.args([
        "-v",
        "error",
        "-show_entries",
        "stream=codec_type,codec_name,pix_fmt:format=duration",
        "-of",
        "json",
    ]);
    command
        .arg(input)
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let output = command.output().await?;
    if !output.status.success() {
        return Err(MediaError::Ffmpeg(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

async fn convert_media(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    mode: DownloadMode,
    probe: &ProbeResult,
    cancel: &CancellationToken,
    progress: &MediaProgress,
) -> Result<(), MediaError> {
    let partial = output.with_file_name(format!(
        "{}.partial.{}",
        output
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("download"),
        output
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("tmp")
    ));
    let _ = fs::remove_file(&partial).await;

    let video = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"));
    let audio = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"));
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-y", "-i"]).arg(input);

    match mode {
        DownloadMode::Video => {
            let video = video.ok_or(MediaError::MissingStream)?;
            command.args(["-map", "0:v:0", "-map", "0:a:0?"]);
            if is_compatible_h264(video) {
                command.args(["-c:v", "copy"]);
            } else {
                command.args([
                    "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
                    "-tag:v", "avc1",
                ]);
            }
            if let Some(audio) = audio {
                if audio.codec_name.as_deref() == Some("aac") {
                    command.args(["-c:a", "copy"]);
                } else {
                    command.args(["-c:a", "aac", "-b:a", "256k"]);
                }
            }
            command.args([
                "-map_metadata",
                "0",
                "-map_chapters",
                "0",
                "-movflags",
                "+faststart",
            ]);
        }
        DownloadMode::AudioOnly => {
            let audio = audio.ok_or(MediaError::MissingStream)?;
            command.args(["-map", "0:a:0", "-vn"]);
            if audio.codec_name.as_deref() == Some("aac") {
                command.args(["-c:a", "copy"]);
            } else {
                command.args(["-c:a", "aac", "-b:a", "256k"]);
            }
            command.args(["-map_metadata", "0", "-movflags", "+faststart"]);
        }
    }
    command
        .args(["-progress", "pipe:1", "-nostats"])
        .arg(&partial);
    configure_child(&mut command, ffmpeg.parent().unwrap_or(Path::new(".")));

    let duration_us = probe
        .format
        .duration
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .map(|seconds| seconds * 1_000_000.0);
    let progress_sink = Arc::clone(progress);
    let stdout_handler = Arc::new(move |line: String| {
        if let Some(value) = line.strip_prefix("out_time_us=")
            && let (Some(duration), Ok(position)) = (duration_us, value.parse::<f64>())
        {
            let fraction = (position / duration).clamp(0.0, 1.0) as f32;
            (progress_sink)(ProgressUpdate {
                phase: JobPhase::Converting,
                fraction: Some(fraction),
                downloaded_bytes: None,
                total_bytes: None,
                speed_bytes_per_second: None,
                eta_seconds: None,
                message: "Converting to a compatible format…".into(),
            });
        }
    });
    let diagnostics = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(30)));
    let diagnostic_sink = Arc::clone(&diagnostics);
    let stderr_handler = Arc::new(move |line: String| {
        let mut lines = diagnostic_sink.lock().expect("diagnostics mutex poisoned");
        if lines.len() == 30 {
            lines.pop_front();
        }
        lines.push_back(line);
    });

    (progress)(ProgressUpdate::message(
        JobPhase::Converting,
        "Creating compatible output…",
    ));
    let status = run_command(command, cancel, stdout_handler, stderr_handler).await?;
    if !status.success() {
        let _ = fs::remove_file(&partial).await;
        let details = diagnostics
            .lock()
            .expect("diagnostics mutex poisoned")
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(MediaError::Ffmpeg(if details.is_empty() {
            status.to_string()
        } else {
            details
        }));
    }

    (progress)(ProgressUpdate::message(
        JobPhase::Finalizing,
        "Finalizing output…",
    ));
    fs::rename(partial, output).await?;
    Ok(())
}

async fn run_command(
    mut command: Command,
    cancel: &CancellationToken,
    stdout_handler: Arc<dyn Fn(String) + Send + Sync>,
    stderr_handler: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<ExitStatus, MediaError> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let process_job = ProcessJob::attach(&child).ok();
    let process_id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing child stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing child stderr"))?;

    let stdout_task = tokio::spawn(read_lines(stdout, stdout_handler));
    let stderr_task = tokio::spawn(read_lines(stderr, stderr_handler));
    let status = tokio::select! {
        status = child.wait() => status?,
        _ = cancel.cancelled() => {
            #[cfg(windows)]
            if let Some(job) = process_job.as_ref() {
                job.terminate();
            } else {
                kill_process_tree(process_id).await;
            }
            #[cfg(not(windows))]
            kill_process_tree(process_id).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(MediaError::Cancelled);
        }
    };
    let _ = stdout_task.await;
    let _ = stderr_task.await;
    Ok(status)
}

async fn read_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    handler: Arc<dyn Fn(String) + Send + Sync>,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        handler(line);
    }
}

fn configure_child(command: &mut Command, tool_directory: &Path) {
    let mut paths = vec![tool_directory.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(value) = std::env::join_paths(paths) {
        command.env("PATH", value);
    }
    hide_console(command);
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(windows)]
async fn kill_process_tree(process_id: Option<u32>) {
    let Some(process_id) = process_id else { return };
    let mut command = Command::new("taskkill");
    command.args(["/PID", &process_id.to_string(), "/T", "/F"]);
    hide_console(&mut command);
    let _ = command.output().await;
}

#[cfg(not(windows))]
async fn kill_process_tree(_process_id: Option<u32>) {}

#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &tokio::process::Child) -> io::Result<Self> {
        use std::{mem::size_of, ptr};
        use windows_sys::Win32::{
            Foundation::GetLastError,
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
        };

        // SAFETY: The structures are initialized, the name is optional, and the child
        // process handle stays valid while this function assigns it to the job.
        unsafe {
            let handle = CreateJobObjectW(ptr::null(), ptr::null());
            if handle.is_null() {
                return Err(io::Error::from_raw_os_error(GetLastError() as i32));
            }
            let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&information as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                windows_sys::Win32::Foundation::CloseHandle(handle);
                return Err(error);
            }
            let process_handle = child.raw_handle().ok_or_else(|| {
                windows_sys::Win32::Foundation::CloseHandle(handle);
                io::Error::other("child process has no Windows handle")
            })? as windows_sys::Win32::Foundation::HANDLE;
            if AssignProcessToJobObject(handle, process_handle) == 0 {
                let error = io::Error::from_raw_os_error(GetLastError() as i32);
                windows_sys::Win32::Foundation::CloseHandle(handle);
                return Err(error);
            }
            Ok(Self(handle))
        }
    }

    fn terminate(&self) {
        // SAFETY: self.0 is a live job handle until Drop.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        // SAFETY: this handle was returned by CreateJobObjectW and is closed once here.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn is_compatible_h264(stream: &ProbeStream) -> bool {
    stream.codec_name.as_deref() == Some("h264")
        && matches!(stream.pix_fmt.as_deref(), Some("yuv420p" | "yuvj420p"))
}

fn parse_ytdlp_progress(value: &str) -> Option<ProgressUpdate> {
    let fields: Vec<_> = value.split('|').collect();
    if fields.len() != 5 {
        return None;
    }
    let percent = fields[0].trim().trim_end_matches('%').parse::<f32>().ok();
    let downloaded = optional_u64(fields[1]);
    let total = optional_u64(fields[2]);
    let speed = optional_u64(fields[3]);
    let eta = optional_u64(fields[4]);
    Some(ProgressUpdate {
        phase: JobPhase::Downloading,
        fraction: percent.map(|number| (number / 100.0).clamp(0.0, 1.0)),
        downloaded_bytes: downloaded,
        total_bytes: total,
        speed_bytes_per_second: speed,
        eta_seconds: eta,
        message: "Downloading media…".into(),
    })
}

fn optional_u64(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("NA") || value.eq_ignore_ascii_case("none") {
        None
    } else {
        value.parse().ok()
    }
}

fn unique_output_path(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    let initial = directory.join(format!("{stem}.{extension}"));
    if !initial.exists() {
        return initial;
    }
    for number in 1..10_000 {
        let candidate = directory.join(format!("{stem} ({number}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-{}.{}", Uuid::new_v4(), extension))
}

pub fn describe_progress(update: &ProgressUpdate) -> String {
    let mut parts = vec![update.message.clone()];
    if update.phase == JobPhase::Downloading {
        if let (Some(done), Some(total)) = (update.downloaded_bytes, update.total_bytes) {
            parts.push(format!("{} / {}", format_bytes(done), format_bytes(total)));
        }
        if let Some(speed) = update.speed_bytes_per_second {
            parts.push(format!("{}/s", format_bytes(speed)));
        }
        if let Some(eta) = update.eta_seconds {
            parts.push(format!("ETA {}:{:02}", eta / 60, eta % 60));
        }
    }
    parts.join(" · ")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_download_progress() {
        let result = parse_ytdlp_progress(" 42.5%|425|1000|50|12").unwrap();
        assert_eq!(result.fraction, Some(0.425));
        assert_eq!(result.downloaded_bytes, Some(425));
        assert_eq!(result.eta_seconds, Some(12));
    }

    #[test]
    fn accepts_only_compatible_h264_pixel_formats() {
        let compatible = ProbeStream {
            codec_type: Some("video".into()),
            codec_name: Some("h264".into()),
            pix_fmt: Some("yuv420p".into()),
        };
        assert!(is_compatible_h264(&compatible));
        let incompatible = ProbeStream {
            pix_fmt: Some("yuv444p10le".into()),
            ..compatible
        };
        assert!(!is_compatible_h264(&incompatible));
    }

    #[test]
    fn creates_a_non_conflicting_name() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("clip.mp4"), b"").unwrap();
        assert_eq!(
            unique_output_path(directory.path(), "clip", "mp4"),
            directory.path().join("clip (1).mp4")
        );
    }
}
