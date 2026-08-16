use crate::{config, model::ActiveToolset};
use fs2::FileExt;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use thiserror::Error;
use tokio::{io::AsyncWriteExt, process::Command};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const YT_DLP_RELEASE_API: &str =
    "https://api.github.com/repos/yt-dlp/yt-dlp-nightly-builds/releases/latest";
const FFMPEG_RELEASE_API: &str =
    "https://api.github.com/repos/yt-dlp/FFmpeg-Builds/releases/latest";
const DENO_RELEASE_API: &str = "https://api.github.com/repos/denoland/deno/releases/latest";

const YT_DLP_ASSET: &str = "yt-dlp.exe";
const YT_DLP_CHECKSUMS: &str = "SHA2-256SUMS";
const FFMPEG_ASSET: &str = "ffmpeg-master-latest-win64-gpl.zip";
const FFMPEG_CHECKSUMS: &str = "checksums.sha256";
const DENO_ASSET: &str = "deno-x86_64-pc-windows-msvc.zip";
const DENO_CHECKSUMS: &str = "deno-x86_64-pc-windows-msvc.zip.sha256sum";

pub type ToolProgress = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("network request failed: {0}")]
    Network(#[from] reqwest::Error),
    #[error("file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("release metadata is invalid: {0}")]
    Metadata(String),
    #[error("archive is invalid: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("checksum verification failed for {0}")]
    Checksum(String),
    #[error("tool validation failed: {0}")]
    Validation(String),
    #[error("another process is updating the tools")]
    UpdateLocked,
    #[error("tool installation was cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedRelease {
    etag: Option<String>,
    release: GithubRelease,
}

impl GithubRelease {
    fn asset(&self, name: &str) -> Result<GithubAsset, ToolError> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .cloned()
            .ok_or_else(|| ToolError::Metadata(format!("release asset {name:?} is missing")))
    }
}

#[derive(Debug, Clone)]
struct RemoteComponent {
    version: String,
    archive: GithubAsset,
    checksums: GithubAsset,
}

#[derive(Debug, Clone)]
pub struct UpdatePlan {
    yt_dlp: RemoteComponent,
    ffmpeg: RemoteComponent,
    deno: RemoteComponent,
    pub update_yt_dlp: bool,
    pub update_ffmpeg: bool,
    pub update_deno: bool,
}

impl UpdatePlan {
    pub fn has_updates(&self) -> bool {
        self.update_yt_dlp || self.update_ffmpeg || self.update_deno
    }

    pub fn required_download_bytes(&self) -> u64 {
        [
            (self.update_yt_dlp, self.yt_dlp.archive.size),
            (self.update_ffmpeg, self.ffmpeg.archive.size),
            (self.update_deno, self.deno.archive.size),
        ]
        .into_iter()
        .filter_map(|(needed, size)| needed.then_some(size))
        .sum()
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.update_yt_dlp {
            parts.push(format!("yt-dlp {}", self.yt_dlp.version));
        }
        if self.update_ffmpeg {
            parts.push(format!("FFmpeg {}", short_version(&self.ffmpeg.version)));
        }
        if self.update_deno {
            parts.push(format!("Deno {}", self.deno.version));
        }
        let mib = self.required_download_bytes() as f64 / 1_048_576.0;
        format!("{} ({mib:.0} MiB download)", parts.join(", "))
    }
}

#[derive(Clone)]
pub struct ToolManager {
    root: PathBuf,
    client: reqwest::Client,
    release_cache: Arc<tokio::sync::Mutex<HashMap<String, CachedRelease>>>,
}

impl ToolManager {
    pub fn new(root: PathBuf) -> Result<Self, ToolError> {
        fs::create_dir_all(root.join("tools"))?;
        fs::create_dir_all(root.join("staging"))?;
        let client = reqwest::Client::builder()
            .user_agent(concat!("yt-dlp-wrapper/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(8))
            .build()?;
        let release_cache = match config::read_json(&root.join("release-cache.json")) {
            Ok(cache) => cache.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "could not read the release cache; using an empty cache");
                HashMap::new()
            }
        };
        Ok(Self {
            root,
            client,
            release_cache: Arc::new(tokio::sync::Mutex::new(release_cache)),
        })
    }

    pub fn load_active(&self) -> Result<Option<ActiveToolset>, ToolError> {
        let path = self.root.join("active-tools.json");
        let Some(mut active) = config::read_json::<ActiveToolset>(&path)? else {
            return Ok(None);
        };
        if active.directory.is_relative() {
            active.directory = self.root.join(&active.directory);
        }
        Ok(self.validate_paths(&active).then_some(active))
    }

    pub async fn check_updates(
        &self,
        active: Option<&ActiveToolset>,
    ) -> Result<UpdatePlan, ToolError> {
        let (yt, ffmpeg, deno) = tokio::try_join!(
            self.release(YT_DLP_RELEASE_API),
            self.release(FFMPEG_RELEASE_API),
            self.release(DENO_RELEASE_API),
        )?;

        let yt_component = RemoteComponent {
            version: yt.tag_name.clone(),
            archive: yt.asset(YT_DLP_ASSET)?,
            checksums: yt.asset(YT_DLP_CHECKSUMS)?,
        };
        let ffmpeg_archive = ffmpeg.asset(FFMPEG_ASSET)?;
        let ffmpeg_component = RemoteComponent {
            version: ffmpeg_archive.updated_at.clone(),
            archive: ffmpeg_archive,
            checksums: ffmpeg.asset(FFMPEG_CHECKSUMS)?,
        };
        let deno_component = RemoteComponent {
            version: deno.tag_name.clone(),
            archive: deno.asset(DENO_ASSET)?,
            checksums: deno.asset(DENO_CHECKSUMS)?,
        };

        Ok(UpdatePlan {
            update_yt_dlp: active.is_none_or(|a| a.yt_dlp_version != yt_component.version),
            update_ffmpeg: active.is_none_or(|a| a.ffmpeg_version != ffmpeg_component.version),
            update_deno: active.is_none_or(|a| a.deno_version != deno_component.version),
            yt_dlp: yt_component,
            ffmpeg: ffmpeg_component,
            deno: deno_component,
        })
    }

    pub async fn install(
        &self,
        plan: &UpdatePlan,
        active: Option<&ActiveToolset>,
        progress: ToolProgress,
        cancel: CancellationToken,
    ) -> Result<ActiveToolset, ToolError> {
        let lock_path = self.root.join("update.lock");
        let lock = File::create(lock_path)?;
        lock.try_lock_exclusive()
            .map_err(|_| ToolError::UpdateLocked)?;

        let stage = self.root.join("staging").join(Uuid::new_v4().to_string());
        fs::create_dir_all(&stage)?;
        let result = self
            .install_into(&stage, plan, active, progress, &cancel)
            .await;
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        result
    }

    async fn install_into(
        &self,
        stage: &Path,
        plan: &UpdatePlan,
        active: Option<&ActiveToolset>,
        progress: ToolProgress,
        cancel: &CancellationToken,
    ) -> Result<ActiveToolset, ToolError> {
        ensure_not_cancelled(cancel)?;
        if plan.update_yt_dlp {
            progress("Downloading yt-dlp nightly…".into());
            let destination = stage.join("yt-dlp.exe");
            self.download(&plan.yt_dlp.archive, &destination, cancel)
                .await?;
            self.verify_from_asset(&destination, &plan.yt_dlp.checksums, YT_DLP_ASSET)
                .await?;
        } else {
            copy_active(active, "yt-dlp.exe", stage)?;
        }

        ensure_not_cancelled(cancel)?;
        if plan.update_ffmpeg {
            progress("Downloading FFmpeg and ffprobe…".into());
            let archive = stage.join("ffmpeg.zip");
            self.download(&plan.ffmpeg.archive, &archive, cancel)
                .await?;
            self.verify_from_asset(&archive, &plan.ffmpeg.checksums, FFMPEG_ASSET)
                .await?;
            let stage_owned = stage.to_path_buf();
            tokio::task::spawn_blocking(move || extract_ffmpeg(&archive, &stage_owned))
                .await
                .map_err(|error| ToolError::Validation(error.to_string()))??;
        } else {
            copy_active(active, "ffmpeg.exe", stage)?;
            copy_active(active, "ffprobe.exe", stage)?;
            copy_optional_active(active, "licences/ffmpeg-license.txt", stage)?;
        }

        ensure_not_cancelled(cancel)?;
        if plan.update_deno {
            progress("Downloading Deno…".into());
            let archive = stage.join("deno.zip");
            self.download(&plan.deno.archive, &archive, cancel).await?;
            self.verify_from_asset(&archive, &plan.deno.checksums, DENO_ASSET)
                .await?;
            let stage_owned = stage.to_path_buf();
            tokio::task::spawn_blocking(move || extract_deno(&archive, &stage_owned))
                .await
                .map_err(|error| ToolError::Validation(error.to_string()))??;
        } else {
            copy_active(active, "deno.exe", stage)?;
            copy_optional_active(active, "licences/deno-license.txt", stage)?;
        }

        ensure_not_cancelled(cancel)?;
        for temporary in [stage.join("ffmpeg.zip"), stage.join("deno.zip")] {
            let _ = fs::remove_file(temporary);
        }

        progress("Validating downloaded tools…".into());
        smoke_test(&stage.join("yt-dlp.exe"), &["--version"]).await?;
        smoke_test(&stage.join("ffmpeg.exe"), &["-version"]).await?;
        smoke_test(&stage.join("ffprobe.exe"), &["-version"]).await?;
        smoke_test(&stage.join("deno.exe"), &["--version"]).await?;

        let id = format!(
            "{}-{}-{}",
            safe_id(&plan.yt_dlp.version),
            safe_id(&short_version(&plan.ffmpeg.version)),
            safe_id(&plan.deno.version)
        );
        let final_directory = self.root.join("tools").join(&id);
        if final_directory.exists() {
            fs::remove_dir_all(&final_directory)?;
        }
        fs::rename(stage, &final_directory)?;

        let relative_directory = PathBuf::from("tools").join(&id);
        let stored = ActiveToolset {
            id,
            yt_dlp_version: plan.yt_dlp.version.clone(),
            ffmpeg_version: plan.ffmpeg.version.clone(),
            deno_version: plan.deno.version.clone(),
            directory: relative_directory,
        };
        config::write_json_atomic(&self.root.join("active-tools.json"), &stored)?;

        let mut active = stored;
        active.directory = self.root.join(&active.directory);
        self.cleanup_old_toolsets(&active.directory);
        Ok(active)
    }

    async fn release(&self, url: &str) -> Result<GithubRelease, ToolError> {
        let cached = self.release_cache.lock().await.get(url).cloned();
        let mut request = self.client.get(url);
        if let Some(etag) = cached.as_ref().and_then(|entry| entry.etag.as_ref()) {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = tokio::time::timeout(std::time::Duration::from_secs(20), request.send())
            .await
            .map_err(|_| ToolError::Metadata("release request timed out".into()))??;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return cached.map(|entry| entry.release).ok_or_else(|| {
                ToolError::Metadata("server returned 304 without cached metadata".into())
            });
        }
        let response = response.error_for_status()?;
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let release: GithubRelease = response.json().await?;
        let mut cache = self.release_cache.lock().await;
        cache.insert(
            url.to_owned(),
            CachedRelease {
                etag,
                release: release.clone(),
            },
        );
        config::write_json_atomic(&self.root.join("release-cache.json"), &*cache)?;
        Ok(release)
    }

    async fn download(
        &self,
        asset: &GithubAsset,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), ToolError> {
        let response = self
            .client
            .get(&asset.browser_download_url)
            .send()
            .await?
            .error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(destination).await?;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Err(ToolError::Cancelled),
                chunk = stream.next() => match chunk {
                    Some(chunk) => file.write_all(&chunk?).await?,
                    None => break,
                }
            }
        }
        file.flush().await?;
        Ok(())
    }

    async fn verify_from_asset(
        &self,
        file: &Path,
        checksums: &GithubAsset,
        expected_name: &str,
    ) -> Result<(), ToolError> {
        let text = self
            .client
            .get(&checksums.browser_download_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let expected = checksum_for(&text, expected_name).ok_or_else(|| {
            ToolError::Metadata(format!("no checksum exists for {expected_name}"))
        })?;
        let path = file.to_path_buf();
        let actual = tokio::task::spawn_blocking(move || sha256_file(&path))
            .await
            .map_err(|error| ToolError::Validation(error.to_string()))??;
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err(ToolError::Checksum(expected_name.into()));
        }
        Ok(())
    }

    fn validate_paths(&self, active: &ActiveToolset) -> bool {
        [
            active.yt_dlp(),
            active.ffmpeg(),
            active.ffprobe(),
            active.deno(),
        ]
        .into_iter()
        .all(|path| path.is_file())
    }

    fn cleanup_old_toolsets(&self, current: &Path) {
        let Ok(entries) = fs::read_dir(self.root.join("tools")) else {
            return;
        };
        let mut directories: Vec<_> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir() && entry.path() != current)
            .collect();
        directories.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());
        while directories.len() > 1 {
            let entry = directories.remove(0);
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn ensure_not_cancelled(cancel: &CancellationToken) -> Result<(), ToolError> {
    if cancel.is_cancelled() {
        Err(ToolError::Cancelled)
    } else {
        Ok(())
    }
}

fn copy_active(
    active: Option<&ActiveToolset>,
    relative: &str,
    stage: &Path,
) -> Result<(), ToolError> {
    let active =
        active.ok_or_else(|| ToolError::Validation(format!("missing cached {relative}")))?;
    let source = active.directory.join(relative);
    let destination = stage.join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

fn copy_optional_active(
    active: Option<&ActiveToolset>,
    relative: &str,
    stage: &Path,
) -> Result<(), ToolError> {
    let Some(active) = active else { return Ok(()) };
    let source = active.directory.join(relative);
    if source.exists() {
        let destination = stage.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn checksum_for(contents: &str, expected_name: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        (name == expected_name && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn extract_ffmpeg(archive: &Path, stage: &Path) -> Result<(), ToolError> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let licence_dir = stage.join("licences");
    fs::create_dir_all(&licence_dir)?;
    let mut found_ffmpeg = false;
    let mut found_ffprobe = false;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(ToolError::Validation(
                "FFmpeg ZIP contains an unsafe path".into(),
            ));
        };
        let Some(name) = enclosed.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let destination = match name.to_ascii_lowercase().as_str() {
            "ffmpeg.exe" => {
                found_ffmpeg = true;
                Some(stage.join("ffmpeg.exe"))
            }
            "ffprobe.exe" => {
                found_ffprobe = true;
                Some(stage.join("ffprobe.exe"))
            }
            "license.txt" | "copying" => Some(licence_dir.join("ffmpeg-license.txt")),
            _ => None,
        };
        if let Some(destination) = destination {
            let mut output = File::create(destination)?;
            io::copy(&mut entry, &mut output)?;
            output.flush()?;
        }
    }
    if !found_ffmpeg || !found_ffprobe {
        return Err(ToolError::Validation(
            "FFmpeg ZIP does not contain ffmpeg.exe and ffprobe.exe".into(),
        ));
    }
    Ok(())
}

fn extract_deno(archive: &Path, stage: &Path) -> Result<(), ToolError> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut found = false;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(ToolError::Validation(
                "Deno ZIP contains an unsafe path".into(),
            ));
        };
        let Some(name) = enclosed.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.eq_ignore_ascii_case("deno.exe") {
            let mut output = File::create(stage.join("deno.exe"))?;
            io::copy(&mut entry, &mut output)?;
            output.flush()?;
            found = true;
        } else if name.eq_ignore_ascii_case("LICENSE.md") {
            fs::create_dir_all(stage.join("licences"))?;
            let mut output = File::create(stage.join("licences/deno-license.txt"))?;
            io::copy(&mut entry, &mut output)?;
        }
    }
    if !found {
        return Err(ToolError::Validation(
            "Deno ZIP does not contain deno.exe".into(),
        ));
    }
    Ok(())
}

async fn smoke_test(executable: &Path, arguments: &[&str]) -> Result<(), ToolError> {
    let output = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|error| {
            ToolError::Validation(format!("could not start {}: {error}", executable.display()))
        })?;
    if !output.status.success() {
        return Err(ToolError::Validation(format!(
            "{} returned {}",
            executable.display(),
            output.status
        )));
    }
    Ok(())
}

fn short_version(value: &str) -> String {
    value.chars().take(19).collect()
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_checksum_formats() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum_for(&format!("{hash}  yt-dlp.exe\n"), "yt-dlp.exe"),
            Some(hash.clone())
        );
        assert_eq!(
            checksum_for(&format!("{hash} *yt-dlp.exe\n"), "yt-dlp.exe"),
            Some(hash)
        );
    }

    #[test]
    fn rejects_wrong_checksum_name() {
        let hash = "b".repeat(64);
        assert_eq!(
            checksum_for(&format!("{hash} other.exe\n"), "yt-dlp.exe"),
            None
        );
    }
}
