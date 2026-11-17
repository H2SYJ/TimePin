use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::core::settings::{load_settings, Settings};
use crate::ipc::SCHEMA_VERSION;

pub const MINIMUM_TOOL_VERSION: &str = "6.0";
const MINIMUM_TOOL_MAJOR: u64 = 6;
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_VERSION_OUTPUT: u64 = 64 * 1024;
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyStatus {
    pub id: String,
    pub available: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub error: Option<String>,
    pub minimum_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    pub schema_version: String,
    pub dependencies: Vec<DependencyStatus>,
}

/// 配置错误时不会回退 PATH，避免实际运行与依赖报告使用不同工具。
pub fn resolve_ffmpeg(app_handle: &AppHandle) -> Option<PathBuf> {
    resolve_ffmpeg_with_settings(&load_settings(app_handle).ok()?)
}

pub fn resolve_ffprobe(app_handle: &AppHandle) -> Option<PathBuf> {
    resolve_ffprobe_with_settings(&load_settings(app_handle).ok()?)
}

pub fn resolve_ffmpeg_with_settings(settings: &Settings) -> Option<PathBuf> {
    resolve_dependency("ffmpeg", settings).ok()
}

pub fn resolve_ffprobe_with_settings(settings: &Settings) -> Option<PathBuf> {
    resolve_dependency("ffprobe", settings).ok()
}

fn executable_names(binary: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{binary}.exe")]
    } else {
        vec![binary.to_string()]
    }
}

fn existing_candidate(dir: &Path, binary: &str) -> Option<PathBuf> {
    executable_names(binary)
        .into_iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn configured_binary(path: &Path, binary: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{binary} 配置必须是绝对路径：{}", path.display()));
    }
    let resolved = if path.is_dir() {
        existing_candidate(path, binary)
    } else {
        path.is_file().then(|| path.to_path_buf())
    };
    resolved.ok_or_else(|| format!("{binary} 配置不存在或未包含可执行文件：{}", path.display()))
}

fn resolve_dependency(binary: &str, settings: &Settings) -> Result<PathBuf, String> {
    let configured = if binary == "ffmpeg" {
        settings.ffmpeg_path.as_ref()
    } else {
        settings.ffprobe_path.as_ref()
    };
    if let Some(path) = configured {
        return configured_binary(path, binary);
    }
    if binary == "ffprobe" {
        if let Some(ffmpeg) = settings.ffmpeg_path.as_ref() {
            // 使用 ffmpegPath 的 bin 目录或可执行文件的同级目录。
            // 显式路径本身损坏或找不到同级 ffprobe 时，不静默回退 PATH。
            if !ffmpeg.is_absolute() {
                return Err("ffmpegPath 必须是绝对路径".to_string());
            }
            let directory = if ffmpeg.is_dir() {
                ffmpeg.clone()
            } else {
                configured_binary(ffmpeg, "ffmpeg")?
                    .parent()
                    .ok_or_else(|| "ffmpegPath 没有父目录".to_string())?
                    .to_path_buf()
            };
            return existing_candidate(&directory, "ffprobe").ok_or_else(|| {
                format!(
                    "配置的 FFmpeg 目录中找不到 ffprobe：{}",
                    directory.display()
                )
            });
        }
    }
    find_in_path(binary).ok_or_else(|| format!("PATH 中找不到 {binary}，请安装或配置工具路径"))
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| existing_candidate(&dir, binary))
}

pub fn check_dependencies(settings: &Settings) -> DependencyReport {
    let dependencies = ["ffmpeg", "ffprobe"]
        .into_iter()
        .map(|binary| match resolve_dependency(binary, settings) {
            Ok(path) => probe_tool(binary, &path),
            Err(error) => DependencyStatus {
                id: binary.to_string(),
                available: false,
                path: if binary == "ffmpeg" {
                    settings.ffmpeg_path.clone()
                } else {
                    settings.ffprobe_path.clone()
                },
                version: None,
                error: Some(error),
                minimum_version: MINIMUM_TOOL_VERSION.to_string(),
            },
        })
        .collect();
    DependencyReport {
        schema_version: SCHEMA_VERSION.to_string(),
        dependencies,
    }
}

/// 只校验自定义的工具（ffmpegPath 隐式配置同目录 ffprobe）。
/// 没有自定义路径时，系统依赖缺失不会阻止正常设置保存。
pub fn validate_configured_tools(settings: &Settings) -> Result<(), String> {
    let mut errors = Vec::new();
    for binary in ["ffmpeg", "ffprobe"] {
        let custom = settings.ffmpeg_path.is_some()
            || (binary == "ffprobe" && settings.ffprobe_path.is_some());
        if !custom {
            continue;
        }
        let result = resolve_dependency(binary, settings)
            .map(|path| probe_tool(binary, &path))
            .and_then(|status| match status.error {
                Some(error) => Err(error),
                None => Ok(status),
            });
        if let Err(error) = result {
            errors.push(format!("{binary}：{error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

pub fn probe_tool(binary: &str, path: &Path) -> DependencyStatus {
    let mut status = DependencyStatus {
        id: binary.to_string(),
        available: false,
        path: Some(path.to_path_buf()),
        version: None,
        error: None,
        minimum_version: MINIMUM_TOOL_VERSION.to_string(),
    };
    let result = validate_executable(path)
        .and_then(|()| run_version_probe(path, VERSION_TIMEOUT))
        .and_then(|output| parse_version_line(binary, &output));
    match result {
        Ok(version) => {
            let major = version_major(&version).expect("parsed numeric version");
            if major >= MINIMUM_TOOL_MAJOR {
                status.available = true;
            } else {
                status.error = Some(format!(
                    "版本 {version} 过旧，要求 >= {MINIMUM_TOOL_VERSION}"
                ));
            }
            status.version = Some(version);
        }
        Err(error) => status.error = Some(error),
    }
    status
}

fn validate_executable(path: &Path) -> Result<(), String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("路径不可读：{error}"))?;
    if !metadata.is_file() {
        return Err("路径不是可执行文件".to_string());
    }
    // Windows 的 .bat/.cmd 会被系统隐式交给命令解释器，明确禁止。
    #[cfg(windows)]
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return Err("请选择 .exe 可执行文件，不支持脚本或批处理".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("文件没有执行权限".to_string());
        }
    }
    Ok(())
}

/// 非阻塞捕获有硬性内存上限，无临时磁盘写入、后台读线程或无限 EOF 等待。
/// 超时/输出超限/读取失败均先终止并回收直接子进程，再返回错误。
fn run_version_probe(path: &Path, timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(path);
    command.arg("-version").stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let (mut child, mut capture) =
        VersionCapture::spawn(&mut command).map_err(|error| format!("启动失败：{error}"))?;
    drop(command);
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    let result = (|| loop {
        capture.drain(&mut bytes)?;
        match child
            .try_wait()
            .map_err(|error| format!("等待进程失败：{error}"))?
        {
            Some(status) => {
                capture.drain(&mut bytes)?;
                if !status.success() {
                    return Err(format!("-version 执行失败：{status}"));
                }
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            None if Instant::now() >= deadline => {
                return Err(format!("-version 执行超时（{} 毫秒）", timeout.as_millis()));
            }
            None => std::thread::sleep(WAIT_POLL_INTERVAL),
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

struct VersionCapture {
    #[cfg(unix)]
    reader: std::os::unix::net::UnixStream,
    #[cfg(windows)]
    stdout: std::process::ChildStdout,
    #[cfg(windows)]
    stderr: std::process::ChildStderr,
}

impl VersionCapture {
    #[cfg(unix)]
    fn spawn(command: &mut Command) -> std::io::Result<(std::process::Child, Self)> {
        use std::os::fd::OwnedFd;
        use std::os::unix::net::UnixStream;

        let (reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        command
            .stdout(Stdio::from(OwnedFd::from(writer.try_clone()?)))
            .stderr(Stdio::from(OwnedFd::from(writer)));
        let child = command.spawn()?;
        Ok((child, Self { reader }))
    }

    #[cfg(windows)]
    fn spawn(command: &mut Command) -> std::io::Result<(std::process::Child, Self)> {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        Ok((child, Self { stdout, stderr }))
    }

    fn drain(&mut self, bytes: &mut Vec<u8>) -> Result<(), String> {
        #[cfg(unix)]
        read_available(&mut self.reader, bytes)?;
        #[cfg(windows)]
        {
            read_available(&mut self.stdout, bytes)?;
            read_available(&mut self.stderr, bytes)?;
        }
        Ok(())
    }
}

fn append_output(bytes: &mut Vec<u8>, output: &[u8]) -> Result<(), String> {
    bytes.extend_from_slice(output);
    if bytes.len() as u64 > MAX_VERSION_OUTPUT {
        Err("-version 输出超过 64 KiB 限制".to_string())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn read_available(reader: &mut impl Read, bytes: &mut Vec<u8>) -> Result<(), String> {
    let mut buffer = [0u8; 4096];
    loop {
        let limit = buffer
            .len()
            .min((MAX_VERSION_OUTPUT + 1) as usize - bytes.len());
        match reader.read(&mut buffer[..limit]) {
            Ok(0) => return Ok(()),
            Ok(size) => append_output(bytes, &buffer[..size])?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("读取版本输出失败：{error}")),
        }
    }
}

#[cfg(windows)]
fn read_available(
    reader: &mut (impl Read + std::os::windows::io::AsRawHandle),
    bytes: &mut Vec<u8>,
) -> Result<(), String> {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn PeekNamedPipe(
            pipe: *mut c_void,
            buffer: *mut c_void,
            buffer_size: u32,
            bytes_read: *mut u32,
            total_available: *mut u32,
            bytes_left: *mut u32,
        ) -> i32;
    }
    let mut buffer = [0u8; 4096];
    loop {
        let mut available = 0u32;
        // SAFETY: reader owns a valid anonymous-pipe read handle; only total_available
        // is written, and every other pointer is null with zero buffer length.
        let ok = unsafe {
            PeekNamedPipe(
                reader.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(109 | 232 | 233)) {
                return Ok(()); // pipe closed by the child
            }
            return Err(format!("读取版本输出失败：{error}"));
        }
        if available == 0 {
            return Ok(());
        }
        let limit = buffer
            .len()
            .min(available as usize)
            .min((MAX_VERSION_OUTPUT + 1) as usize - bytes.len());
        let size = reader
            .read(&mut buffer[..limit])
            .map_err(|error| format!("读取版本输出失败：{error}"))?;
        if size == 0 {
            return Ok(());
        }
        append_output(bytes, &buffer[..size])?;
    }
}

/// 仅接受对应工具的版本 banner 和稳定的数字版本；不从版权年等任意数字猜版本。
fn parse_version_line(binary: &str, output: &str) -> Result<String, String> {
    let line = output
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| "-version 输出为空".to_string())?
        .trim();
    let prefix = format!("{binary} version ");
    let token = line
        .strip_prefix(&prefix)
        .and_then(|rest| rest.split_whitespace().next())
        .ok_or_else(|| format!("输出不是有效的 {binary} 版本信息"))?;
    let token = token.strip_prefix('n').unwrap_or(token);
    let end = token
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(token.len());
    let numeric = &token[..end];
    let suffix = &token[end..];
    let parts: Vec<_> = numeric.split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || part.parse::<u64>().is_err())
        || (!suffix.is_empty() && !suffix.starts_with(['-', '+', '_']))
        || ["-dev", "-rc", "-alpha", "-beta"]
            .iter()
            .any(|prerelease| suffix.to_ascii_lowercase().starts_with(prerelease))
    {
        return Err(format!("无法解析 {binary} 的稳定版本：{token}"));
    }
    Ok(numeric.to_string())
}

fn version_major(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholder(dir: &Path, binary: &str) -> PathBuf {
        let path = dir.join(&executable_names(binary)[0]);
        std::fs::write(&path, b"not executable").unwrap();
        path
    }

    #[test]
    fn resolves_configured_file_directory_and_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg = placeholder(dir.path(), "ffmpeg");
        let ffprobe = placeholder(dir.path(), "ffprobe");
        let mut settings = Settings {
            ffmpeg_path: Some(ffmpeg.clone()),
            ..Settings::default()
        };
        assert_eq!(
            resolve_ffmpeg_with_settings(&settings),
            Some(ffmpeg.clone())
        );
        assert_eq!(
            resolve_ffprobe_with_settings(&settings),
            Some(ffprobe.clone())
        );
        settings.ffmpeg_path = Some(dir.path().to_path_buf());
        assert_eq!(
            resolve_ffmpeg_with_settings(&settings),
            Some(ffmpeg.clone())
        );
        assert_eq!(
            resolve_ffprobe_with_settings(&settings),
            Some(ffprobe.clone())
        );
        std::fs::remove_file(ffmpeg).unwrap();
        assert!(resolve_ffmpeg_with_settings(&settings).is_none());
        assert_eq!(resolve_ffprobe_with_settings(&settings), Some(ffprobe));
    }

    #[test]
    fn explicit_ffprobe_wins_and_broken_paths_do_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join(&executable_names("custom-probe")[0]);
        std::fs::write(&explicit, b"placeholder").unwrap();
        let mut settings = Settings {
            ffmpeg_path: Some(dir.path().join("missing")),
            ffprobe_path: Some(explicit.clone()),
            ..Settings::default()
        };
        assert_eq!(resolve_ffprobe_with_settings(&settings), Some(explicit));
        assert!(resolve_ffmpeg_with_settings(&settings).is_none());
        settings.ffprobe_path = None;
        assert!(resolve_ffprobe_with_settings(&settings).is_none());
        placeholder(dir.path(), "ffmpeg");
        settings.ffmpeg_path = Some(dir.path().to_path_buf());
        assert!(resolve_ffprobe_with_settings(&settings).is_none());
    }

    #[test]
    fn parses_common_stable_versions_and_newer_majors() {
        for (banner, expected) in [
            ("ffmpeg version 6.1.1 Copyright 2024", "6.1.1"),
            ("ffmpeg version n6.1.1-1-gabcdef", "6.1.1"),
            ("ffmpeg version 7.1-full_build-www.gyan.dev", "7.1"),
            ("ffmpeg version 8.0.1-static", "8.0.1"),
            ("ffmpeg version 6.1.1-0+deb12u1", "6.1.1"),
        ] {
            assert_eq!(parse_version_line("ffmpeg", banner).unwrap(), expected);
        }
        assert_eq!(
            parse_version_line("ffprobe", "ffprobe version 7.0").unwrap(),
            "7.0"
        );
    }

    #[test]
    fn rejects_unparseable_banners_and_tool_impersonators() {
        for banner in [
            "fake ffmpeg version 6.1.1",
            "ffprobe version 6.1.1",
            "ffmpeg fake version 6.1.1",
            "ffmpeg version unknown Copyright 2026",
            "ffmpeg version N-12345-gdeadbeef Copyright 2026",
            "ffmpeg version 7",
            "ffmpeg version 7..1",
            "ffmpeg version 7.0-rc1",
            "ffmpeg version 7.0evil",
        ] {
            assert!(parse_version_line("ffmpeg", banner).is_err(), "{banner}");
        }
    }

    #[test]
    fn custom_validation_skips_system_absence_and_rejects_missing_files() {
        assert!(validate_configured_tools(&Settings::default()).is_ok());
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            ffmpeg_path: Some(dir.path().join("missing")),
            ..Settings::default()
        };
        assert!(validate_configured_tools(&settings).is_err());
        let report = check_dependencies(&settings);
        assert_eq!(report.schema_version, "1.0.0");
        assert_eq!(report.dependencies.len(), 2);
        assert!(report.dependencies.iter().all(|status| !status.available));
        assert_eq!(report.dependencies[0].minimum_version, "6.0");
    }

    #[test]
    fn reports_both_tools_with_versioned_camel_case_contract() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            ffmpeg_path: Some(dir.path().join("missing-ffmpeg")),
            ffprobe_path: Some(dir.path().join("missing-ffprobe")),
            ..Settings::default()
        };
        let report = serde_json::to_value(check_dependencies(&settings)).unwrap();
        assert_eq!(report["schemaVersion"], "1.0.0");
        let tools = report["dependencies"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        for (tool, id) in tools.iter().zip(["ffmpeg", "ffprobe"]) {
            assert_eq!(tool["id"], id);
            assert_eq!(tool["available"], false);
            assert_eq!(tool["minimumVersion"], "6.0");
            assert!(tool["path"].is_string());
            assert!(tool["version"].is_null());
            assert!(tool["error"].is_string());
        }
    }

    #[test]
    fn rejects_nonexecutable_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_tool("ffmpeg", dir.path()).error.is_some());
        let path = placeholder(dir.path(), "ffmpeg");
        assert!(!probe_tool("ffmpeg", &path).available);
    }

    #[test]
    fn native_probe_is_bounded_checks_exit_and_reaps_timeout() {
        // A native test fixture exercises the same path on Windows/macOS/Linux.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("fixture.rs");
        std::fs::write(
            &source,
            r#"
use std::io::Write;
fn main() {
    let path = std::env::current_exe().unwrap();
    let name = path.file_stem().unwrap().to_string_lossy();
    if name == "hang" {
        loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
    }
    if name == "flood" {
        let bytes = [b'x'; 8192];
        loop { std::io::stdout().write_all(&bytes).unwrap(); }
    }
    if name == "failed" { std::process::exit(1); }
    if name == "old" { println!("ffmpeg version 5.1.7"); return; }
    if name == "ffprobe" { eprintln!("ffprobe version 6.1.1"); return; }
    println!("ffmpeg version 6.1.1 Copyright test fixture");
}
"#,
        )
        .unwrap();
        let executable = dir.path().join(&executable_names("ffmpeg")[0]);
        let compile = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        assert!(probe_tool("ffmpeg", &executable).available);
        let ffprobe = dir.path().join(&executable_names("ffprobe")[0]);
        std::fs::copy(&executable, &ffprobe).unwrap();
        assert!(probe_tool("ffprobe", &ffprobe).available);
        let custom_pair = Settings {
            ffmpeg_path: Some(dir.path().to_path_buf()),
            ..Settings::default()
        };
        assert!(validate_configured_tools(&custom_pair).is_ok());
        for mode in ["hang", "flood", "failed", "old"] {
            let target = dir.path().join(&executable_names(mode)[0]);
            std::fs::copy(&executable, &target).unwrap();
            let start = Instant::now();
            if mode == "old" {
                let result = probe_tool("ffmpeg", &target);
                assert!(!result.available);
                assert_eq!(result.version.as_deref(), Some("5.1.7"));
            } else {
                let error = run_version_probe(&target, Duration::from_millis(250)).unwrap_err();
                let expected = match mode {
                    "hang" => "超时",
                    "flood" => "超过",
                    _ => "执行失败",
                };
                assert!(error.contains(expected), "{mode}: {error}");
                assert!(start.elapsed() < Duration::from_secs(3));
                // Windows cannot delete a running executable: this also verifies child reaping.
                std::fs::remove_file(&target).unwrap();
            }
        }
    }
}
