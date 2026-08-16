#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod media;
mod model;
mod tools;

use config::AppConfig;
use directories::UserDirs;
use media::MediaError;
use model::{ActiveToolset, DownloadMode, DownloadRequest};
use slint::{ComponentHandle, SharedString, Weak};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tokio_util::sync::CancellationToken;
use tools::{ToolManager, UpdatePlan};

slint::include_modules!();

struct AppRuntime {
    manager: ToolManager,
    config_path: PathBuf,
    config: Mutex<AppConfig>,
    pending_update: AsyncMutex<Option<UpdatePlan>>,
    active_tools: RwLock<Option<ActiveToolset>>,
    cancellation: Mutex<Option<CancellationToken>>,
    completed_file: Mutex<Option<PathBuf>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    let data_root = match config::portable_data_root() {
        Ok(root) => root,
        Err(error) => {
            ui.set_busy(false);
            ui.set_failed(true);
            ui.set_status_text(format!(
                "The application folder is not writable. Extract the application to a writable folder. {error}"
            ).into());
            ui.run()?;
            return Ok(());
        }
    };
    let _log_guard = initialize_logging(&data_root);
    let runtime = Arc::new(Runtime::new()?);
    let manager = ToolManager::new(data_root.clone())?;
    let config_path = data_root.join("config.json");
    let app_config = match config::read_json::<AppConfig>(&config_path) {
        Ok(config) => config.unwrap_or_default(),
        Err(error) => {
            tracing::warn!(%error, "could not read settings; using defaults");
            AppConfig::default()
        }
    };
    let output_directory = app_config
        .output_directory
        .clone()
        .or_else(default_download_directory)
        .unwrap_or_else(|| data_root.clone());
    ui.set_output_folder(output_directory.to_string_lossy().into_owned().into());

    let state = Arc::new(AppRuntime {
        manager,
        config_path,
        config: Mutex::new(app_config),
        pending_update: AsyncMutex::new(None),
        active_tools: RwLock::new(None),
        cancellation: Mutex::new(None),
        completed_file: Mutex::new(None),
    });

    wire_callbacks(&ui, Arc::clone(&runtime), Arc::clone(&state));
    schedule_tool_check(Arc::clone(&runtime), Arc::clone(&state), ui.as_weak());
    ui.run()?;
    if let Some(token) = state
        .cancellation
        .lock()
        .expect("cancellation mutex poisoned")
        .as_ref()
    {
        token.cancel();
    }
    Ok(())
}

fn wire_callbacks(ui: &AppWindow, runtime: Arc<Runtime>, state: Arc<AppRuntime>) {
    let weak = ui.as_weak();
    let browse_state = Arc::clone(&state);
    ui.on_browse_output(move || {
        let current = weak
            .upgrade()
            .map(|ui| PathBuf::from(ui.get_output_folder().to_string()));
        let mut dialog = rfd::FileDialog::new();
        if let Some(directory) = current.filter(|path| path.is_dir()) {
            dialog = dialog.set_directory(directory);
        }
        if let Some(directory) = dialog.pick_folder() {
            if let Some(ui) = weak.upgrade() {
                ui.set_output_folder(directory.to_string_lossy().into_owned().into());
            }
            let mut saved = browse_state.config.lock().expect("config mutex poisoned");
            saved.output_directory = Some(directory);
            if let Err(error) = config::write_json_atomic(&browse_state.config_path, &*saved) {
                tracing::warn!(%error, "could not save the output folder");
            }
        }
    });

    let install_runtime = Arc::clone(&runtime);
    let install_state = Arc::clone(&state);
    let install_weak = ui.as_weak();
    ui.on_install_tools(move || {
        schedule_tool_install(
            Arc::clone(&install_runtime),
            Arc::clone(&install_state),
            install_weak.clone(),
        );
    });

    let check_runtime = Arc::clone(&runtime);
    let check_state = Arc::clone(&state);
    let check_weak = ui.as_weak();
    ui.on_retry_tool_check(move || {
        schedule_tool_check(
            Arc::clone(&check_runtime),
            Arc::clone(&check_state),
            check_weak.clone(),
        );
    });

    ui.on_defer_update({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_update_available(false);
                ui.set_can_install_tools(false);
                ui.set_busy(false);
                ui.set_cancellable(false);
                ui.set_tools_ready(true);
                ui.set_status_text("Ready. The cached tools will be used.".into());
            }
        }
    });

    let download_runtime = Arc::clone(&runtime);
    let download_state = Arc::clone(&state);
    let download_weak = ui.as_weak();
    ui.on_start_download(move |url, audio_only, output_folder| {
        schedule_download(
            Arc::clone(&download_runtime),
            Arc::clone(&download_state),
            download_weak.clone(),
            url.to_string(),
            audio_only,
            PathBuf::from(output_folder.to_string()),
        );
    });

    let cancel_state = Arc::clone(&state);
    ui.on_cancel_download(move || {
        if let Some(token) = cancel_state
            .cancellation
            .lock()
            .expect("cancellation mutex poisoned")
            .as_ref()
        {
            token.cancel();
        }
    });

    let open_state = Arc::clone(&state);
    ui.on_open_folder(move || {
        if let Some(path) = open_state
            .completed_file
            .lock()
            .expect("completed file mutex poisoned")
            .clone()
        {
            open_in_file_manager(&path);
        }
    });
}

fn schedule_tool_check(runtime: Arc<Runtime>, state: Arc<AppRuntime>, weak: Weak<AppWindow>) {
    set_ui(&weak, |ui| {
        ui.set_busy(true);
        ui.set_cancellable(false);
        ui.set_tools_ready(false);
        ui.set_setup_required(false);
        ui.set_update_available(false);
        ui.set_can_install_tools(false);
        ui.set_failed(false);
        ui.set_status_text("Checking required tools…".into());
    });
    runtime.spawn(async move {
        let cached = match state.manager.load_active() {
            Ok(value) => value,
            Err(error) => {
                show_tool_check_failure(&weak, true, &error.to_string());
                return;
            }
        };
        *state.active_tools.write().await = cached.clone();
        match state.manager.check_updates(cached.as_ref()).await {
            Ok(plan) if plan.has_updates() => {
                let first_setup = cached.is_none();
                let summary = plan.summary();
                *state.pending_update.lock().await = Some(plan);
                set_ui(&weak, move |ui| {
                    ui.set_busy(false);
                    ui.set_tools_ready(!first_setup);
                    ui.set_setup_required(first_setup);
                    ui.set_update_available(!first_setup);
                    ui.set_can_install_tools(true);
                    ui.set_update_summary(summary.into());
                    ui.set_status_text(if first_setup {
                        "Install the required tools to continue.".into()
                    } else {
                        "Updates are available.".into()
                    });
                });
            }
            Ok(_) => {
                *state.pending_update.lock().await = None;
                set_ui(&weak, |ui| {
                    ui.set_busy(false);
                    ui.set_tools_ready(true);
                    ui.set_can_install_tools(false);
                    ui.set_status_text("Ready. All tools are current.".into());
                });
            }
            Err(error) if cached.is_some() => {
                tracing::warn!(%error, "update check failed; using cached tools");
                set_ui(&weak, |ui| {
                    ui.set_busy(false);
                    ui.set_tools_ready(true);
                    ui.set_can_install_tools(false);
                    ui.set_status_text(
                        "Could not check for updates. Cached tools are ready.".into(),
                    );
                });
            }
            Err(error) => show_tool_check_failure(&weak, true, &error.to_string()),
        }
    });
}

fn schedule_tool_install(runtime: Arc<Runtime>, state: Arc<AppRuntime>, weak: Weak<AppWindow>) {
    set_ui(&weak, |ui| {
        ui.set_busy(true);
        ui.set_cancellable(true);
        ui.set_tools_ready(false);
        ui.set_can_install_tools(false);
        ui.set_failed(false);
        ui.set_status_text("Preparing tool installation…".into());
    });
    runtime.spawn(async move {
        let cancel = CancellationToken::new();
        *state
            .cancellation
            .lock()
            .expect("cancellation mutex poisoned") = Some(cancel.clone());
        let plan = state.pending_update.lock().await.clone();
        let Some(plan) = plan else {
            *state
                .cancellation
                .lock()
                .expect("cancellation mutex poisoned") = None;
            set_ui(&weak, |ui| {
                ui.set_busy(false);
                ui.set_cancellable(false);
                ui.set_failed(true);
                ui.set_can_install_tools(false);
                ui.set_status_text(
                    "No update information is available. Select Retry check.".into(),
                );
            });
            return;
        };
        let active = state.active_tools.read().await.clone();
        let progress_weak = weak.clone();
        let progress = Arc::new(move |message: String| {
            set_ui(&progress_weak, move |ui| ui.set_status_text(message.into()));
        });
        match state
            .manager
            .install(&plan, active.as_ref(), progress, cancel)
            .await
        {
            Ok(installed) => {
                *state
                    .cancellation
                    .lock()
                    .expect("cancellation mutex poisoned") = None;
                *state.active_tools.write().await = Some(installed);
                *state.pending_update.lock().await = None;
                set_ui(&weak, |ui| {
                    ui.set_busy(false);
                    ui.set_cancellable(false);
                    ui.set_tools_ready(true);
                    ui.set_setup_required(false);
                    ui.set_update_available(false);
                    ui.set_can_install_tools(false);
                    ui.set_failed(false);
                    ui.set_status_text("Tools installed. Ready to download.".into());
                });
            }
            Err(tools::ToolError::Cancelled) => {
                *state
                    .cancellation
                    .lock()
                    .expect("cancellation mutex poisoned") = None;
                let has_cached = active.is_some();
                set_ui(&weak, move |ui| {
                    ui.set_busy(false);
                    ui.set_cancellable(false);
                    ui.set_tools_ready(has_cached);
                    ui.set_setup_required(!has_cached);
                    ui.set_can_install_tools(true);
                    ui.set_status_text("Tool installation cancelled.".into());
                });
            }
            Err(error) => {
                *state
                    .cancellation
                    .lock()
                    .expect("cancellation mutex poisoned") = None;
                let has_cached = active.is_some();
                let details = error.to_string();
                set_ui(&weak, move |ui| {
                    ui.set_busy(false);
                    ui.set_cancellable(false);
                    ui.set_tools_ready(has_cached);
                    ui.set_failed(true);
                    ui.set_setup_required(!has_cached);
                    ui.set_can_install_tools(true);
                    ui.set_status_text("Tool installation failed.".into());
                    ui.set_details_text(details.into());
                });
            }
        }
    });
}

fn schedule_download(
    runtime: Arc<Runtime>,
    state: Arc<AppRuntime>,
    weak: Weak<AppWindow>,
    url: String,
    audio_only: bool,
    output_directory: PathBuf,
) {
    if !is_http_url(&url) {
        set_ui(&weak, |ui| {
            ui.set_failed(true);
            ui.set_status_text("Enter a valid HTTP or HTTPS video URL.".into());
        });
        return;
    }
    let token = CancellationToken::new();
    *state
        .cancellation
        .lock()
        .expect("cancellation mutex poisoned") = Some(token.clone());
    *state
        .completed_file
        .lock()
        .expect("completed file mutex poisoned") = None;
    set_ui(&weak, |ui| {
        ui.set_busy(true);
        ui.set_cancellable(true);
        ui.set_completed(false);
        ui.set_failed(false);
        ui.set_show_details(false);
        ui.set_details_text(SharedString::default());
        ui.set_progress(0.0);
        ui.set_status_text("Preparing download…".into());
    });

    runtime.spawn(async move {
        let Some(tools) = state.active_tools.read().await.clone() else {
            *state
                .cancellation
                .lock()
                .expect("cancellation mutex poisoned") = None;
            set_ui(&weak, |ui| {
                ui.set_busy(false);
                ui.set_cancellable(false);
                ui.set_failed(true);
                ui.set_status_text("Required tools are not available.".into());
            });
            return;
        };
        let request = DownloadRequest {
            url,
            mode: if audio_only {
                DownloadMode::AudioOnly
            } else {
                DownloadMode::Video
            },
            output_directory,
        };
        let progress_weak = weak.clone();
        let progress = Arc::new(move |update: model::ProgressUpdate| {
            let value = update.fraction.unwrap_or(0.0);
            let text = media::describe_progress(&update);
            set_ui(&progress_weak, move |ui| {
                ui.set_progress(value);
                ui.set_status_text(text.into());
            });
        });
        let result = media::download_media(tools, request, token, progress).await;
        *state
            .cancellation
            .lock()
            .expect("cancellation mutex poisoned") = None;
        match result {
            Ok(path) => {
                *state
                    .completed_file
                    .lock()
                    .expect("completed file mutex poisoned") = Some(path.clone());
                set_ui(&weak, move |ui| {
                    ui.set_busy(false);
                    ui.set_cancellable(false);
                    ui.set_completed(true);
                    ui.set_failed(false);
                    ui.set_progress(1.0);
                    ui.set_status_text(format!("Saved {}", path.display()).into());
                });
            }
            Err(MediaError::Cancelled) => set_ui(&weak, |ui| {
                ui.set_busy(false);
                ui.set_cancellable(false);
                ui.set_progress(0.0);
                ui.set_status_text("Download cancelled.".into());
            }),
            Err(error) => {
                let details = error.to_string();
                set_ui(&weak, move |ui| {
                    ui.set_busy(false);
                    ui.set_cancellable(false);
                    ui.set_failed(true);
                    ui.set_progress(0.0);
                    ui.set_status_text("The download failed.".into());
                    ui.set_details_text(details.into());
                });
            }
        }
    });
}

fn set_ui<F>(weak: &Weak<AppWindow>, function: F)
where
    F: FnOnce(AppWindow) + Send + 'static,
{
    let weak = weak.clone();
    let _ = weak.upgrade_in_event_loop(function);
}

fn show_tool_check_failure(weak: &Weak<AppWindow>, setup_required: bool, error: &str) {
    let details = error.to_owned();
    set_ui(weak, move |ui| {
        ui.set_busy(false);
        ui.set_cancellable(false);
        ui.set_tools_ready(false);
        ui.set_setup_required(setup_required);
        ui.set_can_install_tools(false);
        ui.set_failed(true);
        ui.set_update_summary("An internet connection is required for the first setup.".into());
        ui.set_status_text("Could not check required tools.".into());
        ui.set_details_text(details.into());
    });
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn default_download_directory() -> Option<PathBuf> {
    UserDirs::new().and_then(|directories| directories.download_dir().map(Path::to_path_buf))
}

fn initialize_logging(root: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let log_directory = root.join("logs");
    std::fs::create_dir_all(&log_directory).ok()?;
    let appender = tracing_appender::rolling::daily(log_directory, "yt-dlp-wrapper.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let subscriber = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter("info")
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    Some(guard)
}

#[cfg(windows)]
fn open_in_file_manager(path: &Path) {
    use std::os::windows::process::CommandExt;
    let argument = format!("/select,{}", path.display());
    let _ = Command::new("explorer.exe")
        .arg(argument)
        .creation_flags(0x0800_0000)
        .spawn();
}

#[cfg(target_os = "macos")]
fn open_in_file_manager(path: &Path) {
    let _ = Command::new("open").args(["-R"]).arg(path).spawn();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_in_file_manager(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = Command::new("xdg-open").arg(parent).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::is_http_url;

    #[test]
    fn accepts_only_http_video_urls() {
        assert!(is_http_url("https://example.com/video?id=1"));
        assert!(is_http_url("http://example.com/video"));
        assert!(!is_http_url("file:///C:/secret.txt"));
        assert!(!is_http_url("not a URL"));
    }
}
