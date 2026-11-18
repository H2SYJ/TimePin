//! Shared, bounded ZIP extraction. GitHub keeps its required single wrapper;
//! local packages keep their spelling/layout until the manifest root is selected.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) struct ExtractionLimits {
    pub entries: usize,
    pub entry_bytes: u64,
    pub total_bytes: u64,
}

pub(crate) const EXTRACTION_LIMITS: ExtractionLimits = ExtractionLimits {
    entries: 10_000,
    entry_bytes: 128 * 1024 * 1024,
    total_bytes: 512 * 1024 * 1024,
};

#[derive(Clone, Copy)]
pub(crate) enum Layout {
    Preserve,
    StripSingleRoot,
}

/// Validate the original spelling, independently of the host OS's normalization.
/// Also covers Windows device names, ADS, and trailing-dot/space aliases on Unix.
pub(crate) fn safe_component(component: &str) -> bool {
    if component.is_empty()
        || matches!(component, "." | "..")
        || component.ends_with(['.', ' '])
        || component
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return false;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    if let Some(number) = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
    {
        if matches!(
            number,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        ) {
            return false;
        }
    }
    true
}

pub(crate) fn safe_relative_path(name: &str) -> Result<PathBuf, String> {
    if !name.split('/').all(safe_component) {
        return Err(format!("ZIP 包含不安全的路径：{name}"));
    }
    Ok(name.split('/').collect())
}

const INVALID_METADATA: &str = "ZIP 中央目录元数据无效";
const TOO_MANY_ENTRIES: &str = "ZIP 条目数量超过安全限制";
// EOCD + the largest ZIP comment; also bounds ZIP64's forward signature search.
const METADATA_WINDOW: u64 = 22 + u16::MAX as u64;

fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn le32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn le64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_at(
    reader: &mut (impl Read + Seek),
    position: u64,
    bytes: &mut [u8],
) -> Result<(), &'static str> {
    reader
        .seek(SeekFrom::Start(position))
        .and_then(|_| reader.read_exact(bytes))
        .map_err(|_| INVALID_METADATA)
}

fn check_count(count: u64, limits: ExtractionLimits) -> Result<(), &'static str> {
    if count > limits.entries as u64 {
        return Err(TOO_MANY_ENTRIES);
    }
    Ok(())
}

/// zip 0.6 allocates its entry tables before returning from ZipArchive::new.
/// Its spec module is private, so read only fixed-size fields ourselves first.
/// Follow the same first-signature selection as zip (including ZIP64), or reject:
/// accepting a different footer would leave zip's allocation count unchecked.
fn preflight_zip(
    reader: &mut (impl Read + Seek),
    limits: ExtractionLimits,
) -> Result<(), &'static str> {
    let length = reader
        .seek(SeekFrom::End(0))
        .map_err(|_| INVALID_METADATA)?;
    if length < 22 {
        return Err(INVALID_METADATA);
    }
    let tail_start = length.saturating_sub(METADATA_WINDOW);
    let mut tail = vec![0; (length - tail_start) as usize];
    read_at(reader, tail_start, &mut tail)?;
    let end_index = (0..=tail.len() - 22)
        .rev()
        .find(|&index| &tail[index..index + 4] == b"PK\x05\x06")
        .ok_or(INVALID_METADATA)?;
    let end = &tail[end_index..];
    // zip locates the ZIP64 locator relative to EOF, not the found EOCD.
    // Requiring an exact comment length makes both positions agree.
    if 22 + le16(end, 20) as usize != end.len() {
        return Err(INVALID_METADATA);
    }
    let end_position = tail_start
        .checked_add(end_index as u64)
        .ok_or(INVALID_METADATA)?;
    let counts = [le16(end, 8), le16(end, 10)];
    let size = le32(end, 12);
    let offset = le32(end, 16);
    let needs_zip64 = [le16(end, 4), le16(end, 6), counts[0], counts[1]].contains(&u16::MAX)
        || size == u32::MAX
        || offset == u32::MAX;
    for count in counts {
        if count != u16::MAX {
            check_count(count.into(), limits)?;
        }
    }

    let mut locator = [0; 20];
    let locator_position = end_position.checked_sub(20);
    let has_zip64 = if let Some(position) = locator_position {
        read_at(reader, position, &mut locator)?;
        &locator[..4] == b"PK\x06\x07"
    } else {
        false
    };
    let (count, directory_start, directory_size, directory_bound) = if has_zip64 {
        if le32(&locator, 4) != 0 || le32(&locator, 16) != 1 {
            return Err(INVALID_METADATA); // Multi-disk archives are not supported.
        }
        let locator_position = locator_position.ok_or(INVALID_METADATA)?;
        let nominal_position = le64(&locator, 8);
        // zip searches forward from the nominal offset to account for prefixes.
        // A bounded window supports ordinary/prefixed ZIP64 archives without
        // scanning attacker-controlled padding or an unbounded extensible sector.
        let last_position = locator_position.checked_sub(56).ok_or(INVALID_METADATA)?;
        let search_len = last_position
            .checked_sub(nominal_position)
            .and_then(|n| n.checked_add(4))
            .ok_or(INVALID_METADATA)?
            .min(METADATA_WINDOW);
        let mut search = vec![0; search_len as usize];
        read_at(reader, nominal_position, &mut search)?;
        let found = search
            .windows(4)
            .position(|bytes| bytes == b"PK\x06\x06")
            .ok_or(INVALID_METADATA)?;
        let position = nominal_position
            .checked_add(found as u64)
            .ok_or(INVALID_METADATA)?;
        let mut record = [0; 56];
        read_at(reader, position, &mut record)?;
        let record_size = le64(&record, 4);
        if record_size < 44
            || position
                .checked_add(12)
                .and_then(|n| n.checked_add(record_size))
                != Some(locator_position)
            || le32(&record, 16) != 0
            || le32(&record, 20) != 0
        {
            return Err(INVALID_METADATA);
        }
        let count_on_disk = le64(&record, 24);
        let count = le64(&record, 32);
        check_count(count_on_disk, limits)?;
        check_count(count, limits)?;
        let directory_size = le64(&record, 40);
        let directory_offset = le64(&record, 48);
        if count_on_disk != count
            || !matches!(le16(end, 4), 0 | u16::MAX)
            || !matches!(le16(end, 6), 0 | u16::MAX)
            || counts
                .iter()
                .any(|&n| n != u16::MAX && u64::from(n) != count)
            || (size != u32::MAX && u64::from(size) != directory_size)
            || (offset != u32::MAX && u64::from(offset) != directory_offset)
        {
            return Err(INVALID_METADATA);
        }
        let prefix = position
            .checked_sub(nominal_position)
            .ok_or(INVALID_METADATA)?;
        let directory_start = directory_offset
            .checked_add(prefix)
            .ok_or(INVALID_METADATA)?;
        (count, directory_start, directory_size, position)
    } else {
        if needs_zip64 || le16(end, 4) != 0 || le16(end, 6) != 0 || counts[0] != counts[1] {
            return Err(INVALID_METADATA);
        }
        // Match zip's correction for an executable/other prefix before the ZIP.
        let prefix = end_position
            .checked_sub(size.into())
            .and_then(|n| n.checked_sub(offset.into()))
            .ok_or(INVALID_METADATA)?;
        let directory_start = u64::from(offset)
            .checked_add(prefix)
            .ok_or(INVALID_METADATA)?;
        (
            u64::from(counts[0]),
            directory_start,
            u64::from(size),
            end_position,
        )
    };
    check_count(count, limits)?;
    let directory_end = directory_start
        .checked_add(directory_size)
        .filter(|&n| n <= directory_bound)
        .ok_or(INVALID_METADATA)?;
    if count.checked_mul(46).ok_or(INVALID_METADATA)? > directory_size {
        return Err(INVALID_METADATA);
    }
    // Check fixed central headers and skip their variable fields without
    // allocating. zip must not consume entries outside the declared directory.
    let mut position = directory_start;
    for _ in 0..count {
        let header_end = position
            .checked_add(46)
            .filter(|&n| n <= directory_end)
            .ok_or(INVALID_METADATA)?;
        let mut header = [0; 46];
        read_at(reader, position, &mut header)?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(INVALID_METADATA);
        }
        let variable_size = u64::from(le16(&header, 28))
            + u64::from(le16(&header, 30))
            + u64::from(le16(&header, 32));
        position = header_end
            .checked_add(variable_size)
            .filter(|&n| n <= directory_end)
            .ok_or(INVALID_METADATA)?;
    }
    Ok(())
}

pub(crate) fn extract_zip<R: Read + Seek>(
    mut reader: R,
    target: &Path,
    layout: Layout,
    limits: ExtractionLimits,
) -> Result<(), String> {
    preflight_zip(&mut reader, limits).map_err(str::to_owned)?;
    let mut archive = zip::ZipArchive::new(reader).map_err(|_| "解析 ZIP 失败".to_owned())?;
    if archive.len() > limits.entries {
        return Err("ZIP 条目数量超过安全限制".into());
    }
    let mut total = 0;
    let mut root: Option<String> = None;
    let mut explicit = HashSet::new();
    let mut nodes: HashMap<String, (String, bool)> = HashMap::new();
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| "读取 ZIP 条目失败".to_owned())?;
        let original = file.name().to_owned();
        let name = if file.is_dir() {
            original.strip_suffix('/').unwrap_or(&original)
        } else {
            &original
        };
        safe_relative_path(name)?;
        let parts: Vec<_> = name.split('/').collect();
        let unix_mode = file.unix_mode();
        let mode = unix_mode.unwrap_or(0) & 0o170000;
        if !matches!(mode, 0 | 0o100000 | 0o040000)
            || (mode == 0o040000 && !file.is_dir())
            || (mode == 0o100000 && file.is_dir())
        {
            return Err("ZIP 不允许链接或特殊文件".into());
        }
        if !explicit.insert(name.to_uppercase()) {
            return Err("ZIP 包含重复或大小写冲突的路径".into());
        }
        // Include implicit parents, so Foo/a + foo/b cannot alias on Windows.
        for end in 1..=parts.len() {
            let prefix = parts[..end].join("/");
            let is_dir = end < parts.len() || file.is_dir();
            if let Some((spelling, was_dir)) = nodes.get(&prefix.to_uppercase()) {
                if spelling != &prefix || !was_dir || !is_dir {
                    return Err("ZIP 包含大小写冲突或文件/目录冲突".into());
                }
            } else {
                nodes.insert(prefix.to_uppercase(), (prefix, is_dir));
            }
        }
        let relative = match layout {
            Layout::Preserve => safe_relative_path(name)?,
            Layout::StripSingleRoot => {
                if let Some(root) = &root {
                    if root != parts[0] {
                        return Err("ZIP 顶层目录不一致".into());
                    }
                } else {
                    root = Some(parts[0].to_owned());
                }
                if parts.len() == 1 {
                    if file.is_dir() {
                        continue;
                    }
                    return Err("ZIP 文件不在仓库根目录内".into());
                }
                parts[1..].iter().collect()
            }
        };
        let out = target.join(relative);
        if file.is_dir() {
            if file.size() != 0 {
                return Err("ZIP 目录包含异常内容".into());
            }
            std::fs::create_dir_all(out).map_err(|e| format!("创建插件子目录失败：{e}"))?;
            continue;
        }
        let allowed = limits
            .entry_bytes
            .min(limits.total_bytes.saturating_sub(total));
        if file.size() > allowed {
            return Err("ZIP 解压大小超过安全限制".into());
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建插件子目录失败：{e}"))?;
        }
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&out)
            .map_err(|e| format!("创建插件文件失败：{e}"))?;
        copy_bounded(&mut file, &mut output, allowed).map(|copied| total += copied)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // Preserve executable/readable bits for binaries without honoring
            // special bits or group/other writes. Always retain owner read/write.
            // Never apply archive directory modes: parents must stay traversable.
            let permissions = std::fs::Permissions::from_mode(sanitized_file_mode(unix_mode));
            output
                .set_permissions(permissions)
                .map_err(|e| format!("设置插件文件权限失败：{e}"))?;
        }
        output
            .sync_all()
            .map_err(|e| format!("写入插件文件失败：{e}"))?;
    }
    Ok(())
}

#[cfg(any(unix, test))]
fn sanitized_file_mode(mode: Option<u32>) -> u32 {
    0o600 | (mode.unwrap_or(0o644) & 0o155)
}

fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl std::io::Write,
    allowed: u64,
) -> Result<u64, String> {
    let copied = std::io::copy(&mut reader.take(allowed.saturating_add(1)), writer)
        .map_err(|e| format!("读取或写入 ZIP 内容失败：{e}"))?;
    if copied > allowed {
        return Err("ZIP 解压大小超过安全限制".into());
    }
    Ok(copied)
}

#[cfg(test)]
mod tests;
