//! Local ZIP import: validate in an isolated transaction before replacing an install.
//! Never executes a package or installs dependencies. The caller supplies the
//! single `<exe_dir>/plugins` root; package preparation remains unit-testable.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::archive::{extract_zip, safe_component, safe_relative_path, Layout, EXTRACTION_LIMITS};
use super::manifest::PluginManifest;

pub const IMPORT_SCHEMA_VERSION: &str = "1.0";
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BASE64_BYTES: u64 = MAX_ARCHIVE_BYTES.div_ceil(3) * 4;
// Shared by local packages and GitHub activation, never by network downloads.
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn install_lock_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    INSTALL_LOCK
        .lock()
        .map_err(|_| "插件安装锁异常，请重启应用".to_owned())
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ImportPluginSource {
    Path(String),
    Bytes(EncodedPackage),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EncodedPackage {
    pub base64: String,
    pub file_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPluginRequest {
    pub source: ImportPluginSource,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
}

fn default_schema_version() -> String {
    IMPORT_SCHEMA_VERSION.to_owned()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPluginResult {
    pub schema_version: &'static str,
    pub name: String,
    pub version: String,
    pub replaced: bool,
}

/// Fully validate before invoking the lifecycle callback. The install lock includes
/// that callback (stop → activate → reload); it must not install recursively.
pub fn import_package(
    request: ImportPluginRequest,
    plugin_root: &Path,
    on_validated: impl FnOnce(PreparedInstall) -> Result<ImportPluginResult, String>,
) -> Result<ImportPluginResult, String> {
    let _guard = install_lock_guard()?;
    if request.schema_version != IMPORT_SCHEMA_VERSION {
        return Err(format!(
            "不支持的插件导入 schemaVersion：{}",
            request.schema_version
        ));
    }
    crate::core::ensure_plugin_dir(plugin_root)?;
    reject_link_ancestors(plugin_root)?;
    let bytes = read_source(request.source)?;
    let install = prepare_install(&bytes, plugin_root)?;
    on_validated(install)
}

fn check_zip_extension(name: &str) -> Result<(), String> {
    let supported = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("zip") || s.eq_ignore_ascii_case("tppkg"));
    if !supported {
        return Err("只支持导入 .tppkg 或 .zip 插件包".into());
    }
    Ok(())
}

fn check_archive_length(length: u64) -> Result<(), String> {
    if length > MAX_ARCHIVE_BYTES {
        return Err("ZIP 压缩包大小超过 128 MiB 安全限制".into());
    }
    Ok(())
}

fn check_base64_length(length: u64) -> Result<(), String> {
    if length > MAX_BASE64_BYTES {
        return Err("ZIP Base64 编码长度超过 128 MiB 压缩包安全限制".into());
    }
    Ok(())
}

fn read_source(source: ImportPluginSource) -> Result<Vec<u8>, String> {
    let bytes = match source {
        ImportPluginSource::Path(path) => {
            check_zip_extension(&path)?;
            let file = fs::File::open(&path).map_err(|e| format!("读取插件 ZIP 失败：{e}"))?;
            let metadata = file
                .metadata()
                .map_err(|e| format!("读取 ZIP 属性失败：{e}"))?;
            if !metadata.is_file() {
                return Err("插件 ZIP 路径必须指向普通文件".into());
            }
            check_archive_length(metadata.len())?;
            let mut bytes = Vec::new();
            file.take(MAX_ARCHIVE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("读取插件 ZIP 失败：{e}"))?;
            bytes
        }
        ImportPluginSource::Bytes(encoded) => {
            if !safe_component(&encoded.file_name) {
                return Err("ZIP 文件名不合法".into());
            }
            check_zip_extension(&encoded.file_name)?;
            check_base64_length(encoded.base64.len() as u64)?;
            STANDARD
                .decode(encoded.base64)
                .map_err(|_| "ZIP Base64 内容无效".to_owned())?
        }
    };
    check_archive_length(bytes.len() as u64)?;
    Ok(bytes)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not just symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn reject_link_ancestors(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|e| format!("检查插件目录失败：{e}"))?;
        if is_link(&metadata) {
            return Err(
                "插件安装目录不允许符号链接或 Windows reparse point，请检查可执行文件旁的 plugins 目录".into(),
            );
        }
    }
    Ok(())
}

struct Transaction {
    directory: PathBuf,
    staging: PathBuf,
}

impl Transaction {
    fn create(primary: &Path) -> Result<Self, String> {
        let workspace = primary.join(".timepin-installs");
        let created_workspace = match fs::create_dir(&workspace) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(format!("创建插件事务目录失败：{e}")),
        };
        reject_link_ancestors(&workspace)?;
        let directory = workspace.join(uuid::Uuid::new_v4().to_string());
        if let Err(error) = fs::create_dir(&directory) {
            if created_workspace {
                let _ = fs::remove_dir(&workspace);
            }
            return Err(format!("创建插件事务失败：{error}"));
        }
        let transaction = Self {
            staging: directory.join("staging"),
            directory,
        };
        fs::create_dir(&transaction.staging).map_err(|e| format!("创建插件暂存目录失败：{e}"))?;
        Ok(transaction)
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        // Only the newly-created extraction tree is recursively removed. A backup
        // is never deleted (including when rollback fails). Discovery cannot see it.
        let _ = fs::remove_dir_all(&self.staging);
        let _ = fs::remove_dir(&self.directory);
        if let Some(workspace) = self.directory.parent() {
            let _ = fs::remove_dir(workspace);
        }
    }
}

fn manifest_root(staging: &Path) -> Result<PathBuf, String> {
    if staging.join("plugin.json").is_file() || staging.join("manifest.json").is_file() {
        return Ok(staging.to_path_buf());
    }
    let mut entries = fs::read_dir(staging).map_err(|e| format!("读取插件暂存目录失败：{e}"))?;
    let root = entries
        .next()
        .transpose()
        .map_err(|e| e.to_string())?
        .filter(|entry| entry.path().is_dir())
        .ok_or_else(|| {
            "ZIP 根目录或唯一顶层包装目录中必须包含 plugin.json / manifest.json".to_owned()
        })?;
    if entries.next().is_some() {
        return Err("ZIP 必须是根平铺插件或唯一顶层包装目录，不能包含多个顶层项目".into());
    }
    Ok(root.path())
}

fn validated_manifest(root: &Path) -> Result<PluginManifest, String> {
    // Same plugin.json-before-manifest.json precedence as normal local discovery.
    let manifest = PluginManifest::load(root)?;
    let errors = manifest.validate();
    if !errors.is_empty() {
        return Err(format!("插件清单校验失败：{}", errors.join("；")));
    }
    if !safe_component(&manifest.name) {
        return Err("插件名称不能作为安全目录名（不允许 Windows 保留名）".into());
    }
    let entry = root.join(safe_relative_path(&manifest.entry)?);
    if !entry.is_file() {
        return Err("插件入口文件不存在或不是包内普通文件".into());
    }
    let canonical_root = root.canonicalize().map_err(|e| e.to_string())?;
    let canonical_entry = entry.canonicalize().map_err(|e| e.to_string())?;
    if !canonical_entry.starts_with(canonical_root) {
        return Err("插件入口必须位于插件包内".into());
    }
    Ok(manifest)
}

pub(crate) fn install_target(primary: &Path, name: &str) -> Result<PathBuf, String> {
    // The recognition sidecar shares this root but has an independent process
    // lifecycle. Ordinary plugin installs must never replace its fixed directory.
    if name.eq_ignore_ascii_case("recognition") {
        return Err("recognition 是内置识别服务保留目录，不能通过插件安装覆盖".into());
    }
    // Apply Windows-style identity on every OS to avoid duplicate installs which
    // would alias on Windows. Never follow links when inspecting an old target.
    let mut matches = fs::read_dir(primary)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
        });
    let first = matches.next();
    if matches.next().is_some() {
        return Err("插件安装目标存在多个大小写冲突目录，请先手动整理".into());
    }
    Ok(first
        .map(|entry| entry.path())
        .unwrap_or_else(|| primary.join(name)))
}

/// Owns validated staging until the caller has stopped processes under its
/// lifecycle lock. Dropping it without activation only removes staging.
pub struct PreparedInstall {
    transaction: Transaction,
    root: PathBuf,
    target: PathBuf,
    manifest: PluginManifest,
}

impl PreparedInstall {
    pub fn activate(self) -> Result<ImportPluginResult, String> {
        let backup = self.transaction.directory.join("backup");
        let replaced = switch_install(&self.root, &self.target, &backup)?;
        Ok(ImportPluginResult {
            schema_version: IMPORT_SCHEMA_VERSION,
            name: self.manifest.name,
            version: self.manifest.version,
            replaced,
        })
    }
}

fn prepare_install(bytes: &[u8], primary: &Path) -> Result<PreparedInstall, String> {
    let transaction = Transaction::create(primary)?;
    extract_zip(
        Cursor::new(bytes),
        &transaction.staging,
        Layout::Preserve,
        EXTRACTION_LIMITS,
    )?;
    let root = manifest_root(&transaction.staging)?;
    let manifest = validated_manifest(&root)?;
    let target = install_target(primary, &manifest.name)?;
    validate_target(&target)?;
    reject_same_root_alias(primary, &target, &manifest.name)?;
    Ok(PreparedInstall {
        transaction,
        root,
        target,
        manifest,
    })
}

/// Other direct children of the install root can own manifest.name too; unordered
/// discovery could resolve them before this install or the two would fight.
pub(crate) fn reject_same_root_alias(
    primary: &Path,
    target: &Path,
    name: &str,
) -> Result<(), String> {
    let canonical_target = target.canonicalize();
    let entries = fs::read_dir(primary).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let canonical = match path.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !path.is_dir() || canonical_target.as_ref().is_ok_and(|t| *t == canonical) {
            continue;
        }
        if PluginManifest::load(&path).is_ok_and(|manifest| manifest.name == name) {
            return Err(format!(
                "插件目录内另一目录 {} 已声明同名插件 {name}，导入结果可能无法解析；请移除或改名该目录后重试，原安装未修改",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_target(target: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if is_link(&metadata) || !metadata.is_dir() => {
            Err("插件安装目标不是普通目录或属于链接/reparse point，已保留原目标".into())
        }
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("检查旧插件失败：{e}")),
    }
}

pub(crate) fn switch_install(staging: &Path, target: &Path, backup: &Path) -> Result<bool, String> {
    // Recheck immediately before moving the target. No remove_dir_all(target),
    // even if the previous install has a corrupt/missing manifest.
    let replaced = validate_target(target)?;
    reject_link_ancestors(
        target
            .parent()
            .ok_or_else(|| "插件目标目录无效".to_owned())?,
    )?;
    if replaced {
        fs::rename(target, backup).map_err(|e| format!("备份旧插件失败，原安装未修改：{e}"))?;
    }
    if let Err(error) = fs::rename(staging, target) {
        if replaced && fs::rename(backup, target).is_err() {
            return Err(format!("插件切换失败：{error}；旧安装已保留在 .timepin-installs 事务 backup 中，请手动恢复"));
        }
        return Err(format!("插件切换失败，原安装已保留或恢复：{error}"));
    }
    Ok(replaced)
}

#[cfg(test)]
mod tests;
