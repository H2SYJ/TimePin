//! Pinned FFmpeg/ffprobe installation into `dependencies/ffmpeg` next to the executable.
//!
//! Downloaded bytes are hashed BEFORE decompression and never executed; only
//! after the compressed SHA-256 matches the reviewed b6.1.1 release manifest do
//! we decompress into a fresh per-install directory. The two binary paths are
//! then persisted via `update_settings`, preserving every other settings field.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use uuid::Uuid;

use crate::core::settings::{update_settings, Settings};
use crate::core::tools::{check_dependencies, DependencyReport};
use crate::ipc::SCHEMA_VERSION;

/// 前端监听的事件名：依赖安装进度。
pub const INSTALL_PROGRESS_EVENT: &str = "dependency://install-progress";

const RELEASE: &str = "b6.1.1";
const RELEASE_URL: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1";
const MAX_COMPRESSED_BYTES: u64 = 96 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ATTRIBUTION_BYTES: u64 = 1024 * 1024;

static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDependencies {
    pub schema_version: String,
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
    pub report: DependencyReport,
}

/// Stage of an in-flight install. Serialized as a lowercase string for the
/// wire contract (`"download" | "extract" | "validate" | "completed" | "failed"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyInstallStage {
    Download,
    Extract,
    Validate,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyInstallProgress {
    pub schema_version: String,
    pub task_id: String,
    pub stage: DependencyInstallStage,
    pub progress: f64,
    pub message: String,
    #[serde(default)]
    pub error: Option<String>,
}

fn progress(
    task_id: &str,
    stage: DependencyInstallStage,
    value: f64,
    message: &str,
    error: Option<String>,
) -> DependencyInstallProgress {
    DependencyInstallProgress {
        schema_version: SCHEMA_VERSION.to_string(),
        task_id: task_id.to_string(),
        stage,
        progress: value.clamp(0.0, 1.0),
        message: message.to_string(),
        error,
    }
}

/// 依赖安装根目录：应用可执行文件旁的 `dependencies/ffmpeg`。
///
/// 供 `AdvancedSettingsInfo.installDirectory` 展示使用 —— 返回 PathBuf 而非
/// Result，因为该字段只是给 UI 显示路径的字符串；解析失败时退回 `"."`，
/// 绝不因此阻止设置面板打开。
///
/// 实际安装路径必须经过 `install_root_strict()`，以保证二进制永不写到 CWD。
pub fn install_directory(app_handle: &AppHandle) -> PathBuf {
    install_root_from_exe(app_handle).unwrap_or_else(|_| PathBuf::from("."))
}

/// 安装时必须使用的严格版本：任何解析失败都返回错误，绝不静默写入 CWD。
fn install_root_strict(app_handle: &AppHandle) -> Result<PathBuf, String> {
    install_root_from_exe(app_handle)
}

/// Resolve `dependencies/ffmpeg` next to the running executable.
fn install_root_from_exe(_app_handle: &AppHandle) -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "无法定位可执行文件目录".to_string())?;
    Ok(exe_dir.join("dependencies").join("ffmpeg"))
}

/// Process-wide install mutex. Acquired before any network or settings write;
/// released on all exits including panics via RAII.
struct InstallGuard;

impl InstallGuard {
    fn acquire() -> Result<Self, String> {
        INSTALL_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "已有依赖安装任务正在运行，请等待完成后重试".to_string())
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALL_RUNNING.store(false, Ordering::Release);
    }
}

#[derive(Debug, Clone, Copy)]
struct Source {
    suffix: &'static str,
    ffmpeg_sha256: &'static str,
    ffprobe_sha256: &'static str,
    windows: bool,
    emulated: bool,
}

impl Source {
    fn binary_name(&self, tool: &str) -> String {
        if self.windows {
            format!("{tool}.exe")
        } else {
            tool.to_string()
        }
    }

    fn binary_url(&self, tool: &str) -> String {
        format!("{RELEASE_URL}/{tool}-{}.gz", self.suffix)
    }
}

fn source_for(os: &str, arch: &str) -> Result<Source, String> {
    let (suffix, ffmpeg_sha256, ffprobe_sha256) = match (os, arch) {
        ("windows", "x86_64" | "aarch64") => (
            "win32-x64",
            "8883a3dffbd0a16cf4ef95206ea05283f78908dbfb118f73c83f4951dcc06d77",
            "f309e6223ad89d2fe54bccd420a7709b66fd27540674e92309578ed491a43c8d",
        ),
        ("macos", "x86_64") => (
            "darwin-x64",
            "929b375c1182d956c51f7ac25e0b2b0411fb01f6f407aa15c9758efeb4242106",
            "d4da574d6e2e197bd259b47d69cf262df9e312af24ad960444f6d806d3d4c186",
        ),
        ("macos", "aarch64") => (
            "darwin-arm64",
            "8923876afa8db5585022d7860ec7e589af192f441c56793971276d450ed3bbfa",
            "d986a8ec7b030899fe66a8a288ed809a3543338705a3ce178cfb85869c5d80be",
        ),
        ("linux", "x86_64") => (
            "linux-x64",
            "bfe8a8fc511530457b528c48d77b5737527b504a3797a9bc4866aeca69c2dffa",
            "25d9b6ccb05e3d9de9e04e31e2506d8dd7f9f0418981965ac6df12e8d3afd067",
        ),
        ("linux", "aarch64") => (
            "linux-arm64",
            "754a678672298bc68156adff58aa7385a592c2b30b1d0ae8750c45c915c4bac0",
            "2ab6aba60ee84412dff9188720703376cb4e7aaf7e0b5e43aa8249f2acae5bf8",
        ),
        _ => {
            return Err(format!(
                "暂不支持自动安装 {os}/{arch}，请手动安装 FFmpeg 6.0+ 并在高级设置中指定 ffmpegPath/ffprobePath"
            ));
        }
    };
    Ok(Source {
        suffix,
        ffmpeg_sha256,
        ffprobe_sha256,
        windows: os == "windows",
        emulated: os == "windows" && arch == "aarch64",
    })
}

fn download_client() -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("下载重定向次数过多")
            } else if attempt.url().scheme() != "https" {
                attempt.error("拒绝非 HTTPS 下载重定向")
            } else {
                attempt.follow()
            }
        }))
        .user_agent(concat!(
            "TimePin/",
            env!("CARGO_PKG_VERSION"),
            " dependency-installer"
        ))
        .build()
        .map_err(|error| format!("初始化 HTTPS 下载器失败：{error}"))
}

fn create_new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("无法创建文件 {}：{error}", path.display()))
}

/// Stream `input` → `output` while enforcing `maximum` even when the source
/// lies about its length (chunked transfer, decompression bomb).
fn copy_bounded(
    input: &mut impl Read,
    output: &mut impl Write,
    maximum: u64,
    mut on_progress: impl FnMut(u64),
) -> Result<u64, String> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    let mut last_notified = 0_u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("读取失败：{error}"))?;
        if read == 0 {
            return Ok(copied);
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| "文件大小溢出".to_string())?;
        if copied > maximum {
            return Err(format!("文件超过 {maximum} 字节上限"));
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入失败：{error}"))?;
        if copied - last_notified >= 256 * 1024 {
            on_progress(copied);
            last_notified = copied;
        }
    }
}

fn download_file(
    client: &Client,
    url: &str,
    destination: &Path,
    maximum: u64,
    mut on_progress: impl FnMut(f64),
) -> Result<(), String> {
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("下载失败（{url}）：{error}"))?;
    if let Some(total) = response.content_length() {
        if total > maximum {
            return Err(format!("下载文件超过 {maximum} 字节上限（{url}）"));
        }
    }
    let total = response.content_length();
    let mut file = create_new_file(destination)?;
    let written = copy_bounded(&mut response, &mut file, maximum, |bytes| {
        let denom = total.unwrap_or(maximum).max(1);
        on_progress((bytes as f64 / denom as f64).min(0.99));
    })?;
    file.sync_all()
        .map_err(|error| format!("写入下载文件失败：{error}"))?;
    if written == 0 {
        return Err(format!("下载得到空文件（{url}）"));
    }
    on_progress(1.0);
    Ok(())
}

/// Hash the COMPRESSED bytes on disk. The digest must match the reviewed
/// b6.1.1 manifest before any decompression or execution is allowed.
fn verify_compressed_checksum(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|error| format!("读取压缩文件失败：{error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取文件元数据失败：{error}"))?;
    if metadata.len() > MAX_COMPRESSED_BYTES {
        return Err("压缩文件超过大小上限".to_string());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("校验文件读取失败：{error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != expected {
        return Err(format!(
            "SHA-256 校验失败（{}）：期望 {expected}，实际 {actual}",
            path.display()
        ));
    }
    Ok(())
}

/// Decompress `compressed` → `destination`. The gzip header filename is never
/// consulted; `destination` is always the predetermined name chosen by the
/// caller.
fn extract_verified_gzip(
    compressed: &Path,
    destination: &Path,
    expected_checksum: &str,
    maximum: u64,
) -> Result<(), String> {
    verify_compressed_checksum(compressed, expected_checksum)?;
    let file = File::open(compressed).map_err(|error| format!("打开压缩文件失败：{error}"))?;
    let mut decoder = GzDecoder::new(file);
    let mut output = create_new_file(destination)?;
    let written = copy_bounded(&mut decoder, &mut output, maximum, |_| {})?;
    if written == 0 {
        return Err("压缩包解压后为空".to_string());
    }
    output
        .sync_all()
        .map_err(|error| format!("解压文件写入失败：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("设置执行权限失败：{error}"))?;
    }
    Ok(())
}

/// Download both binaries, the upstream LICENSE/README, and write SOURCE.txt
/// attribution into `staging`.
fn prepare_installation<F: Fn(DependencyInstallProgress)>(
    client: &Client,
    staging: &Path,
    source: Source,
    emit: &F,
    task_id: &str,
) -> Result<(), String> {
    for (index, (tool, checksum)) in [
        ("ffmpeg", source.ffmpeg_sha256),
        ("ffprobe", source.ffprobe_sha256),
    ]
    .into_iter()
    .enumerate()
    {
        let base = index as f64 * 0.42;
        let compressed = staging.join(format!("{tool}.gz"));
        download_file(
            client,
            &source.binary_url(tool),
            &compressed,
            MAX_COMPRESSED_BYTES,
            |fraction| {
                emit(progress(
                    task_id,
                    DependencyInstallStage::Download,
                    base + fraction * 0.34,
                    &format!("正在下载 {tool}（{RELEASE}）"),
                    None,
                ));
            },
        )?;
        emit(progress(
            task_id,
            DependencyInstallStage::Extract,
            base + 0.35,
            &format!("校验并解压 {tool}"),
            None,
        ));
        extract_verified_gzip(
            &compressed,
            &staging.join(source.binary_name(tool)),
            checksum,
            MAX_EXTRACTED_BYTES,
        )?;
        fs::remove_file(&compressed).map_err(|error| format!("清理压缩文件失败：{error}"))?;
        emit(progress(
            task_id,
            DependencyInstallStage::Extract,
            base + 0.42,
            &format!("{tool} 解压完成"),
            None,
        ));
    }
    // Bring the release's own LICENSE/README along; never execute them.
    for (idx, name) in ["LICENSE", "README"].iter().enumerate() {
        emit(progress(
            task_id,
            DependencyInstallStage::Download,
            0.86 + idx as f64 * 0.02,
            &format!("下载上游 {name}"),
            None,
        ));
        let url = format!("{RELEASE_URL}/{}.{name}", source.suffix);
        download_file(
            client,
            &url,
            &staging.join(name),
            MAX_ATTRIBUTION_BYTES,
            |_| {},
        )?;
    }
    let attribution = format!(
        "FFmpeg / ffprobe from eugeneware/ffmpeg-static\nRelease: {RELEASE}\nPlatform: {}\nSource: {RELEASE_URL}/\nCompressed SHA-256 (ffmpeg): {}\nCompressed SHA-256 (ffprobe): {}\nSee LICENSE and README for upstream licensing and build information.\n",
        source.suffix, source.ffmpeg_sha256, source.ffprobe_sha256
    );
    fs::write(staging.join("SOURCE.txt"), attribution)
        .map_err(|error| format!("写入 SOURCE.txt 失败：{error}"))?;
    Ok(())
}

#[derive(Debug)]
struct BinaryPaths {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl BinaryPaths {
    fn at(dir: &Path, source: Source) -> Self {
        Self {
            ffmpeg: dir.join(source.binary_name("ffmpeg")),
            ffprobe: dir.join(source.binary_name("ffprobe")),
        }
    }
}

/// Probe both tools at `paths` via the shared validator; failure here means
/// the freshly-installed files are unusable and must not be adopted.
fn validate_pair(paths: &BinaryPaths) -> Result<DependencyReport, String> {
    let settings = Settings {
        ffmpeg_path: Some(paths.ffmpeg.clone()),
        ffprobe_path: Some(paths.ffprobe.clone()),
        ..Settings::default()
    };
    let report = check_dependencies(&settings);
    if report.dependencies.len() != 2 || report.dependencies.iter().any(|d| !d.available) {
        let detail = report
            .dependencies
            .iter()
            .filter_map(|d| d.error.as_deref())
            .collect::<Vec<_>>()
            .join("；");
        return Err(format!("新工具未通过版本检查：{detail}"));
    }
    Ok(report)
}

/// Staging + final directories. On drop, any directory that was not adopted is
/// removed; pre-existing installs are never touched.
struct InstallWorkspace {
    staging: PathBuf,
    final_directory: PathBuf,
    adopted: bool,
}

impl InstallWorkspace {
    fn new(root: &Path, source: Source) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|error| format!("创建工具目录失败：{error}"))?;
        let id = Uuid::new_v4();
        let staging = root.join(format!(".staging-{id}"));
        let final_directory = root.join(format!("{RELEASE}-{}-{id}", source.suffix));
        fs::create_dir(&staging).map_err(|error| format!("创建 staging 目录失败：{error}"))?;
        Ok(Self {
            staging,
            final_directory,
            adopted: false,
        })
    }

    fn promote(&self) -> Result<(), String> {
        if self.final_directory.exists() {
            return Err("目标安装目录已存在".to_string());
        }
        fs::rename(&self.staging, &self.final_directory)
            .map_err(|error| format!("完成安装目录失败：{error}"))
    }
}

impl Drop for InstallWorkspace {
    fn drop(&mut self) {
        if self.staging.exists() {
            if let Err(error) = fs::remove_dir_all(&self.staging) {
                log::warn!("清理 staging 目录失败：{error}");
            }
        }
        if !self.adopted && self.final_directory.exists() {
            if let Err(error) = fs::remove_dir_all(&self.final_directory) {
                log::warn!("清理未采用目录失败：{error}");
            }
        }
    }
}

fn install_transaction(
    root: &Path,
    source: Source,
    prepare: impl FnOnce(&Path) -> Result<(), String>,
    mut validate: impl FnMut(&BinaryPaths) -> Result<DependencyReport, String>,
    adopt: impl FnOnce(&BinaryPaths) -> Result<(), String>,
) -> Result<(BinaryPaths, DependencyReport), String> {
    let mut workspace = InstallWorkspace::new(root, source)?;
    prepare(&workspace.staging)?;
    validate(&BinaryPaths::at(&workspace.staging, source))?;
    workspace.promote()?;
    let paths = BinaryPaths::at(&workspace.final_directory, source);
    // Re-check both tools at their FINAL path before adopting.
    let report = validate(&paths)?;
    adopt(&paths)?;
    workspace.adopted = true;
    Ok((paths, report))
}

/// Public entry point. Blocking; the Tauri command runs this on `spawn_blocking`.
/// Every failure emits a `failed` event so the frontend always sees an outcome.
pub fn install_dependencies<F>(
    app_handle: &AppHandle,
    task_id: &str,
    emit: F,
) -> Result<InstalledDependencies, String>
where
    F: Fn(DependencyInstallProgress),
{
    let result = (|| {
        let _guard = InstallGuard::acquire()?;
        if task_id.is_empty() || task_id.len() > 128 {
            return Err("task_id 长度需在 1–128 之间".to_string());
        }
        let source = source_for(std::env::consts::OS, std::env::consts::ARCH)?;
        let root = install_root_strict(app_handle)?;
        let note = if source.emulated {
            "（Windows ARM64 使用 x64 仿真版本）"
        } else {
            ""
        };
        emit(progress(
            task_id,
            DependencyInstallStage::Download,
            0.0,
            &format!("从 {RELEASE} 安装 {}{note}", source.suffix),
            None,
        ));
        let client = download_client()?;
        let (paths, report) = install_transaction(
            &root,
            source,
            |staging| prepare_installation(&client, staging, source, &emit, task_id),
            |paths| {
                emit(progress(
                    task_id,
                    DependencyInstallStage::Validate,
                    0.94,
                    "验证 FFmpeg/ffprobe 版本",
                    None,
                ));
                validate_pair(paths)
            },
            |paths| {
                update_settings(app_handle, |settings| {
                    settings.ffmpeg_path = Some(paths.ffmpeg.clone());
                    settings.ffprobe_path = Some(paths.ffprobe.clone());
                })
                .map(|_| ())
                .map_err(|error| format!("写入设置失败：{error}"))
            },
        )?;
        emit(progress(
            task_id,
            DependencyInstallStage::Completed,
            1.0,
            &format!("FFmpeg/ffprobe 安装完成{note}"),
            None,
        ));
        Ok(InstalledDependencies {
            schema_version: SCHEMA_VERSION.to_string(),
            ffmpeg_path: paths.ffmpeg,
            ffprobe_path: paths.ffprobe,
            report,
        })
    })();
    if let Err(error) = &result {
        emit(progress(
            task_id,
            DependencyInstallStage::Failed,
            0.0,
            "依赖安装失败；原有配置保持不变",
            Some(error.clone()),
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, GzBuilder};
    use std::cell::{Cell, RefCell};
    use std::io::Cursor;

    fn fixture(payload: &[u8]) -> (Vec<u8>, String) {
        // A hostile header path must never affect the predetermined output path.
        let mut encoder = GzBuilder::new()
            .filename("../../outside.exe")
            .write(Vec::new(), Compression::default());
        encoder.write_all(payload).expect("fixture write");
        let bytes = encoder.finish().expect("fixture gzip");
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        (bytes, checksum)
    }

    fn fake_report() -> DependencyReport {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": "1.0.0",
            "dependencies": [
                {"id": "ffmpeg", "available": true, "path": null, "version": "6.1.1", "error": null, "minimumVersion": "6.0"},
                {"id": "ffprobe", "available": true, "path": null, "version": "6.1.1", "error": null, "minimumVersion": "6.0"}
            ]
        }))
        .expect("report fixture")
    }

    fn prepare_fixture(staging: &Path, source: Source) -> Result<(), String> {
        let (bytes, checksum) = fixture(b"offline fixture");
        for tool in ["ffmpeg", "ffprobe"] {
            let compressed = staging.join(format!("{tool}.gz"));
            fs::write(&compressed, &bytes).map_err(|e| e.to_string())?;
            extract_verified_gzip(
                &compressed,
                &staging.join(source.binary_name(tool)),
                &checksum,
                4096,
            )?;
            fs::remove_file(compressed).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn dir_count(path: &Path) -> usize {
        fs::read_dir(path).expect("read dir").count()
    }

    #[test]
    fn source_mapping_pinned_unsupported_actionable() {
        for (os, arch, suffix) in [
            ("windows", "x86_64", "win32-x64"),
            ("windows", "aarch64", "win32-x64"),
            ("macos", "x86_64", "darwin-x64"),
            ("macos", "aarch64", "darwin-arm64"),
            ("linux", "x86_64", "linux-x64"),
            ("linux", "aarch64", "linux-arm64"),
        ] {
            let s = source_for(os, arch).expect("source");
            assert_eq!(s.suffix, suffix);
            assert_eq!(s.ffmpeg_sha256.len(), 64);
            assert_eq!(s.ffprobe_sha256.len(), 64);
            assert_ne!(s.ffmpeg_sha256, s.ffprobe_sha256);
            assert_eq!(s.emulated, os == "windows" && arch == "aarch64");
            assert_eq!(
                s.binary_url("ffprobe"),
                format!("{RELEASE_URL}/ffprobe-{suffix}.gz")
            );
        }
        assert!(source_for("linux", "riscv64").is_err());
        assert!(source_for("windows", "x86").is_err());
    }

    #[test]
    fn verified_gzip_ignores_header_filename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (bytes, checksum) = fixture(b"ok");
        let compressed = dir.path().join("b.gz");
        let target = dir.path().join("ffmpeg");
        fs::write(&compressed, bytes).expect("write");
        extract_verified_gzip(&compressed, &target, &checksum, 1024).expect("extract");
        assert_eq!(fs::read(target).expect("read"), b"ok");
        assert_eq!(dir_count(dir.path()), 2);
    }

    #[test]
    fn checksum_mismatch_rejects_before_output_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (bytes, _) = fixture(b"x");
        let compressed = dir.path().join("b.gz");
        let target = dir.path().join("ffmpeg");
        fs::write(&compressed, bytes).expect("write");
        let err = extract_verified_gzip(&compressed, &target, &"0".repeat(64), 1024).unwrap_err();
        assert!(err.contains("SHA-256"));
        assert!(!target.exists());
    }

    #[test]
    fn truncated_or_corrupt_gzip_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (original, _) = fixture(b"integrity check payload");
        for (i, mut bytes) in [
            original[..original.len().saturating_sub(5)].to_vec(),
            original.clone(),
        ]
        .into_iter()
        .enumerate()
        {
            if i == 1 {
                let last = bytes.len() - 1;
                bytes[last] ^= 0xff;
            }
            let compressed = dir.path().join(format!("{i}.gz"));
            let target = dir.path().join(format!("ffmpeg-{i}"));
            let checksum = format!("{:x}", Sha256::digest(&bytes));
            fs::write(&compressed, bytes).expect("write");
            assert!(extract_verified_gzip(&compressed, &target, &checksum, 1024).is_err());
        }
    }

    #[test]
    fn extraction_and_stream_copy_enforce_limits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (bytes, checksum) = fixture(&vec![b'x'; 64 * 1024]);
        let compressed = dir.path().join("b.gz");
        let target = dir.path().join("ffmpeg");
        fs::write(&compressed, bytes).expect("write");
        let err = extract_verified_gzip(&compressed, &target, &checksum, 1024).unwrap_err();
        assert!(err.contains("上限"));
        assert!(fs::metadata(&target).expect("meta").len() <= 1024);
        let mut out = Vec::new();
        assert!(copy_bounded(&mut Cursor::new(vec![1; 100]), &mut out, 10, |_| {}).is_err());
        let mut exact = Vec::new();
        assert_eq!(
            copy_bounded(&mut Cursor::new(vec![1; 10]), &mut exact, 10, |_| {}).expect("exact"),
            10
        );
    }

    #[test]
    fn failure_paths_cleanup_and_never_adopt() {
        let root = tempfile::tempdir().expect("tempdir");
        let source = source_for("windows", "x86_64").expect("source");
        let old = root.path().join("old");
        fs::create_dir(&old).expect("old");
        fs::write(old.join("ffmpeg.exe"), b"old").expect("write");
        for step in 0..4 {
            let adopted = Cell::new(false);
            let calls = Cell::new(0);
            let r = install_transaction(
                root.path(),
                source,
                |staging| {
                    prepare_fixture(staging, source)?;
                    if step == 0 {
                        Err("injected prepare".into())
                    } else {
                        Ok(())
                    }
                },
                |_| {
                    calls.set(calls.get() + 1);
                    if (step == 1 && calls.get() == 1) || (step == 2 && calls.get() == 2) {
                        Err("injected validate".into())
                    } else {
                        Ok(fake_report())
                    }
                },
                |_| {
                    if step == 3 {
                        return Err("injected save".into());
                    }
                    adopted.set(true);
                    Ok(())
                },
            );
            assert!(r.is_err());
            assert!(!adopted.get());
            assert_eq!(dir_count(root.path()), 1);
        }
    }

    #[test]
    fn success_rechecks_then_adopts_in_unique_dir() {
        let root = tempfile::tempdir().expect("tempdir");
        let source = source_for("windows", "x86_64").expect("source");
        let validated = RefCell::new(Vec::new());
        let adopted = Cell::new(0);
        let (paths, _) = install_transaction(
            root.path(),
            source,
            |staging| prepare_fixture(staging, source),
            |p| {
                validated.borrow_mut().push(p.ffmpeg.clone());
                Ok(fake_report())
            },
            |_| {
                adopted.set(adopted.get() + 1);
                Ok(())
            },
        )
        .expect("install");
        assert_eq!(adopted.get(), 1);
        assert_eq!(validated.borrow().len(), 2);
        assert_ne!(validated.borrow()[0], validated.borrow()[1]);
        assert!(paths.ffmpeg.is_file() && paths.ffprobe.is_file());
        assert!(paths
            .ffmpeg
            .parent()
            .expect("parent")
            .file_name()
            .expect("name")
            .to_string_lossy()
            .starts_with("b6.1.1-win32-x64-"));
    }

    #[test]
    fn concurrency_guard_blocks_second() {
        let guard = InstallGuard::acquire().expect("first");
        assert!(InstallGuard::acquire().is_err());
        drop(guard);
        assert!(InstallGuard::acquire().is_ok());
    }

    #[test]
    fn progress_payload_shape() {
        let json = serde_json::to_value(progress(
            "t1",
            DependencyInstallStage::Failed,
            0.5,
            "x",
            Some("boom".into()),
        ))
        .expect("json");
        assert_eq!(json["schemaVersion"], "1.0.0");
        assert_eq!(json["taskId"], "t1");
        assert_eq!(json["stage"], "failed");
        assert_eq!(json["error"], "boom");
    }
}
