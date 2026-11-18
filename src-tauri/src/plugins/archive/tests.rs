use super::*;
use std::io::{self, Cursor, Write};
use zip::write::FileOptions;

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn zip(entries: &[(&str, &[u8])], comment: &str) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.set_comment(comment);
    for (name, contents) in entries {
        writer.start_file(*name, FileOptions::default()).unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn eocd(on_disk: u16, count: u16, size: u32, offset: u32) -> Vec<u8> {
    let mut end = vec![0; 22];
    end[..4].copy_from_slice(b"PK\x05\x06");
    put16(&mut end, 8, on_disk);
    put16(&mut end, 10, count);
    put32(&mut end, 12, size);
    put32(&mut end, 16, offset);
    end
}

fn zip64_tail(on_disk: u64, count: u64, size: u64, offset: u64, position: u64) -> Vec<u8> {
    let mut record = vec![0; 56 + 20];
    record[..4].copy_from_slice(b"PK\x06\x06");
    put64(&mut record, 4, 44);
    put16(&mut record, 12, 45);
    put16(&mut record, 14, 45);
    put64(&mut record, 24, on_disk);
    put64(&mut record, 32, count);
    put64(&mut record, 40, size);
    put64(&mut record, 48, offset);
    record[56..60].copy_from_slice(b"PK\x06\x07");
    put64(&mut record, 64, position);
    put32(&mut record, 72, 1);
    record.extend(eocd(u16::MAX, u16::MAX, u32::MAX, u32::MAX));
    record
}

fn as_zip64(mut bytes: Vec<u8>, comment: &str) -> Vec<u8> {
    let position = bytes.len() - 22 - comment.len();
    let end = &bytes[position..];
    let mut tail = zip64_tail(
        le16(end, 8).into(),
        le16(end, 10).into(),
        le32(end, 12).into(),
        le32(end, 16).into(),
        position as u64,
    );
    put16(&mut tail, 76 + 20, comment.len() as u16);
    tail.extend(comment.as_bytes());
    bytes.truncate(position);
    bytes.extend(tail);
    bytes
}

/// Supply arbitrary logical padding without allocating it. A second EOF seek
/// means ZipArchive::new was entered, and fails the test before any allocation.
struct TailReader {
    bytes: Vec<u8>,
    padding: u64,
    position: u64,
    end_seeks: usize,
}

impl Read for TailReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = self.padding + self.bytes.len() as u64;
        let count = length
            .saturating_sub(self.position)
            .min(buffer.len() as u64) as usize;
        buffer[..count].fill(0);
        if self.position + count as u64 > self.padding {
            let start = self.padding.saturating_sub(self.position) as usize;
            let source = self.position.saturating_sub(self.padding) as usize;
            buffer[start..count].copy_from_slice(&self.bytes[source..source + count - start]);
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for TailReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => {
                if n == 0 {
                    self.end_seeks += 1;
                    assert_eq!(self.end_seeks, 1, "ZipArchive reached before rejection");
                }
                i128::from(self.padding) + self.bytes.len() as i128 + i128::from(n)
            }
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))?;
        Ok(self.position)
    }
}

fn reject_before_archive(bytes: Vec<u8>, padding: u64, error: &str) {
    let target = tempfile::tempdir().unwrap();
    let reader = TailReader {
        bytes,
        padding,
        position: 0,
        end_seeks: 0,
    };
    assert_eq!(
        extract_zip(reader, target.path(), Layout::Preserve, EXTRACTION_LIMITS).unwrap_err(),
        error
    );
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
}

#[test]
fn excessive_ordinary_counts_rejected_before_zip_archive() {
    let excessive = EXTRACTION_LIMITS.entries as u16 + 1;
    // The logical EOCD position exceeds the count, bypassing zip 0.6's guard.
    for (on_disk, total) in [(excessive, excessive), (excessive, 1), (1, excessive)] {
        reject_before_archive(
            eocd(on_disk, total, 0, 0),
            u64::from(excessive) + 1,
            TOO_MANY_ENTRIES,
        );
    }
}

#[test]
fn excessive_zip64_counts_rejected_before_zip_archive() {
    for excessive in [
        EXTRACTION_LIMITS.entries as u64 + 1,
        u64::from(u32::MAX) + 1,
        u64::MAX,
    ] {
        // Even the 64-bit count guard can be bypassed with sparse logical padding.
        let padding = excessive.min(u64::from(u32::MAX) + 1) + 1;
        for (on_disk, count) in [(excessive, excessive), (excessive, 1), (1, excessive)] {
            reject_before_archive(
                zip64_tail(on_disk, count, 0, 0, padding),
                padding,
                TOO_MANY_ENTRIES,
            );
        }
    }
    // zip uses a locator even when the ordinary EOCD did not use sentinels.
    let mut bytes = zip64_tail(10_001, 10_001, 0, 0, 20_000);
    bytes.truncate(76);
    bytes.extend(eocd(1, 1, 0, 0));
    reject_before_archive(bytes, 20_000, TOO_MANY_ENTRIES);
}

#[test]
fn malformed_or_truncated_eocd_rejected_before_zip_archive() {
    for bytes in [vec![], vec![0; 21], vec![0; 50]] {
        reject_before_archive(bytes, 0, INVALID_METADATA);
    }
    let end = eocd(0, 0, 0, 0);
    for length in 0..end.len() {
        reject_before_archive(end[..length].to_vec(), 0, INVALID_METADATA);
    }
    let mut missing_comment = end.clone();
    put16(&mut missing_comment, 20, 1);
    reject_before_archive(missing_comment, 0, INVALID_METADATA);
    let mut trailing_bytes = end.clone();
    trailing_bytes.push(0);
    reject_before_archive(trailing_bytes, 0, INVALID_METADATA);
    let mut multi_disk = end;
    put16(&mut multi_disk, 4, 1);
    reject_before_archive(multi_disk, 0, INVALID_METADATA);
    reject_before_archive(eocd(1, 0, 46, 0), 46, INVALID_METADATA);
}

#[test]
fn malformed_or_truncated_zip64_metadata_rejected_before_zip_archive() {
    reject_before_archive(eocd(u16::MAX, u16::MAX, 0, 0), 0, INVALID_METADATA);
    let tail = zip64_tail(0, 0, 0, 0, 0);
    // Truncate every byte of both the locator and ZIP64 EOCD, retaining EOCD.
    for length in 0..76 {
        let mut bytes = tail[..length].to_vec();
        bytes.extend_from_slice(&tail[76..]);
        reject_before_archive(bytes, 0, INVALID_METADATA);
    }
    for (offset, value) in [(4, 43), (4, u64::MAX), (64, u64::MAX)] {
        let mut bytes = tail.clone();
        put64(&mut bytes, offset, value);
        reject_before_archive(bytes, 0, INVALID_METADATA);
    }
    for (offset, value) in [(16, 1), (20, 1), (60, 1), (72, 2)] {
        let mut bytes = tail.clone();
        put32(&mut bytes, offset, value);
        reject_before_archive(bytes, 0, INVALID_METADATA);
    }
    let mut mismatched = tail.clone();
    put16(&mut mismatched, 76 + 8, 1);
    put16(&mut mismatched, 76 + 10, 1);
    reject_before_archive(mismatched, 0, INVALID_METADATA);
    let mut fake_footer = tail;
    // Do not skip a later EOCD signature in a comment and validate another one.
    put16(&mut fake_footer, 76 + 20, 23);
    let mut fake_end = eocd(0, 0, 0, 0);
    put16(&mut fake_end, 20, 10);
    fake_footer.extend(fake_end);
    fake_footer.push(0);
    reject_before_archive(fake_footer, 0, INVALID_METADATA);
}

#[test]
fn central_directory_bounds_and_overflow_rejected_before_zip_archive() {
    for bytes in [eocd(0, 0, 1, 0), eocd(0, 0, 0, 1), eocd(1, 1, 0, 0)] {
        reject_before_archive(bytes, 0, INVALID_METADATA);
    }
    for (size, offset) in [(1, 0), (0, 1), (u64::MAX, 1), (1, u64::MAX)] {
        reject_before_archive(zip64_tail(0, 0, size, offset, 0), 0, INVALID_METADATA);
    }
    // Prefix correction must not overflow directory_offset + prefix.
    reject_before_archive(zip64_tail(0, 0, 0, u64::MAX, 0), 1, INVALID_METADATA);
    let normal = zip(&[("main.py", b"entry")], "");
    let central = normal.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    let mut excessive_name = normal.clone();
    put16(&mut excessive_name, central + 28, u16::MAX);
    reject_before_archive(excessive_name, 0, INVALID_METADATA);
    let mut wrong_signature = normal;
    wrong_signature[central] = 0;
    reject_before_archive(wrong_signature, 0, INVALID_METADATA);
}

#[test]
fn normal_zip_comments_prefixes_and_layouts_remain_compatible() {
    for comment in [
        String::new(),
        "ordinary ZIP comment".into(),
        "x".repeat(u16::MAX as usize),
    ] {
        for zip64 in [false, true] {
            for prefix in [b"".as_slice(), b"executable prefix".as_slice()] {
                let mut bytes = zip(&[("repo-sha/main.py", b"entry")], &comment);
                if zip64 {
                    bytes = as_zip64(bytes, &comment);
                }
                let mut prefixed = prefix.to_vec();
                prefixed.extend(bytes);
                for (layout, path) in [
                    (Layout::Preserve, "repo-sha/main.py"),
                    (Layout::StripSingleRoot, "main.py"),
                ] {
                    let target = tempfile::tempdir().unwrap();
                    extract_zip(
                        Cursor::new(&prefixed),
                        target.path(),
                        layout,
                        EXTRACTION_LIMITS,
                    )
                    .unwrap();
                    assert_eq!(std::fs::read(target.path().join(path)).unwrap(), b"entry");
                }
            }
        }
    }
}

#[test]
fn empty_zip_and_zip64_extensible_sector_remain_compatible() {
    for bytes in [zip(&[], ""), as_zip64(zip(&[], "comment"), "comment")] {
        preflight_zip(&mut Cursor::new(bytes), EXTRACTION_LIMITS).unwrap();
    }
    let comment = "comment";
    let mut bytes = as_zip64(zip(&[("root/main.py", b"entry")], comment), comment);
    let record = bytes.len() - comment.len() - 22 - 20 - 56;
    put64(&mut bytes, record + 4, 60);
    bytes.splice(record + 56..record + 56, [0; 16]);
    let target = tempfile::tempdir().unwrap();
    extract_zip(
        Cursor::new(bytes),
        target.path(),
        Layout::StripSingleRoot,
        EXTRACTION_LIMITS,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(target.path().join("main.py")).unwrap(),
        b"entry"
    );
}

#[test]
fn zip64_signature_search_is_bounded() {
    let mut tail = zip64_tail(0, 0, 0, 0, 0);
    put64(&mut tail, 64, 0);
    reject_before_archive(tail, METADATA_WINDOW + 1, INVALID_METADATA);
}

#[test]
fn paths_aliases_links_and_layout_checks_still_apply() {
    for entries in [
        vec![("../escape", b"bad".as_slice())],
        vec![("CON.txt", b"bad".as_slice())],
        vec![("a/name.", b"bad".as_slice())],
        vec![("a", b"one".as_slice()), ("A", b"two".as_slice())],
        vec![("sub/a", b"one".as_slice()), ("Sub/b", b"two".as_slice())],
        vec![("a", b"file".as_slice()), ("a/b", b"child".as_slice())],
    ] {
        let target = tempfile::tempdir().unwrap();
        assert!(extract_zip(
            Cursor::new(zip(&entries, "")),
            target.path(),
            Layout::Preserve,
            EXTRACTION_LIMITS
        )
        .is_err());
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("link", "../escape", FileOptions::default())
        .unwrap();
    let target = tempfile::tempdir().unwrap();
    assert!(extract_zip(
        writer.finish().unwrap(),
        target.path(),
        Layout::Preserve,
        EXTRACTION_LIMITS
    )
    .is_err());
    let bytes = zip(&[("root/a", b"one"), ("other/b", b"two")], "");
    assert!(extract_zip(
        Cursor::new(bytes),
        target.path(),
        Layout::StripSingleRoot,
        EXTRACTION_LIMITS
    )
    .is_err());
}

#[test]
fn executable_mode_sanitization_strips_unsafe_bits() {
    for (archive, expected) in [
        (None, 0o644),
        (Some(0o107777), 0o755),
        (Some(0o100700), 0o700),
        (Some(0o100444), 0o644),
        (Some(0), 0o600),
    ] {
        assert_eq!(sanitized_file_mode(archive), expected);
    }
}

#[cfg(unix)]
#[test]
fn unix_executable_bits_restored_without_unsafe_modes_or_directory_chmod() {
    use std::os::unix::fs::PermissionsExt;

    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_directory("root/bin/", FileOptions::default().unix_permissions(0o600))
        .unwrap();
    for (name, mode) in [
        ("runtime", 0o7777),
        ("read-only", 0o444),
        ("private", 0o700),
    ] {
        writer
            .start_file(
                format!("root/bin/{name}"),
                FileOptions::default().unix_permissions(mode),
            )
            .unwrap();
        writer.write_all(b"binary").unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    // FileOptions strips special bits, so put them back in the central record.
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    let runtime = central
        + 46
        + le16(&bytes, central + 28) as usize
        + le16(&bytes, central + 30) as usize
        + le16(&bytes, central + 32) as usize;
    put32(&mut bytes, runtime + 38, 0o107777 << 16);
    let target = tempfile::tempdir().unwrap();
    extract_zip(
        Cursor::new(bytes),
        target.path(),
        Layout::StripSingleRoot,
        EXTRACTION_LIMITS,
    )
    .unwrap();
    for (name, mode) in [("runtime", 0o755), ("read-only", 0o644), ("private", 0o700)] {
        let path = target.path().join("bin").join(name);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            mode
        );
        assert_eq!(std::fs::read(path).unwrap(), b"binary");
    }
    let directory_mode = std::fs::metadata(target.path().join("bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(
        directory_mode & 0o100,
        0,
        "archive directory modes must not remove traversal"
    );
}

#[test]
fn streaming_limit_checks_actual_bytes() {
    assert!(copy_bounded(&mut &b"1234"[..], &mut Vec::new(), 3).is_err());
    assert_eq!(
        copy_bounded(&mut &b"123"[..], &mut Vec::new(), 3).unwrap(),
        3
    );
}
