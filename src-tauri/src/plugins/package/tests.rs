use super::*;
use crate::plugins::archive::{ExtractionLimits, Layout};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use zip::write::FileOptions;

fn manifest(name: &str, version: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "protocolVersion": "1.0.0", "name": name, "version": version,
        "description": "test", "author": "test", "entry": "main.py", "runtime": "python",
        "capabilities": ["metadata", "videoFrames", "fileWrite"],
        "parametersSchema": {}, "outputSchema": {}
    }))
    .unwrap()
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        if name.ends_with('/') {
            writer.add_directory(*name, FileOptions::default()).unwrap();
        } else {
            writer.start_file(*name, FileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

fn package(version: &str) -> Vec<u8> {
    zip(&[
        ("plugin.json", &manifest("demo", version)),
        ("main.py", version.as_bytes()),
    ])
}

fn request(bytes: &[u8]) -> ImportPluginRequest {
    serde_json::from_value(serde_json::json!({
        "source": {"base64": STANDARD.encode(bytes), "fileName": "plugin.ZiP"}
    }))
    .unwrap()
}

fn import_with_reload(
    request: ImportPluginRequest,
    root: &Path,
    reload: impl FnOnce(),
) -> Result<ImportPluginResult, String> {
    super::import_package(request, root, |install| {
        let result = install.activate()?;
        reload();
        Ok(result)
    })
}

fn import(bytes: &[u8], root: &Path) -> Result<ImportPluginResult, String> {
    import_with_reload(request(bytes), root, || {})
}

#[test]
fn imports_path_and_base64_with_camel_case_contract_and_default_version() {
    for name in [
        "plugin.zip",
        "plugin.ZIP",
        "plugin.ZiP",
        "plugin.tppkg",
        "plugin.TPPKG",
    ] {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let file = source.path().join(name);
        fs::write(&file, package("1.2.3")).unwrap();
        let request: ImportPluginRequest =
            serde_json::from_value(serde_json::json!({"source": file.to_str().unwrap()})).unwrap();
        assert_eq!(request.schema_version, "1.0");
        let calls = AtomicUsize::new(0);
        let result = import_with_reload(request, root.path(), || {
            calls.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "schemaVersion": "1.0", "name": "demo", "version": "1.2.3", "replaced": false
            })
        );
        assert_eq!(
            fs::read(root.path().join("demo/main.py")).unwrap(),
            b"1.2.3"
        );
        assert!(!root.path().join(".timepin-installs").exists());

        let bytes_root = tempfile::tempdir().unwrap();
        let encoded: ImportPluginRequest = serde_json::from_value(serde_json::json!({
            "source": { "base64": STANDARD.encode(package("2")), "fileName": name }
        }))
        .unwrap();
        assert_eq!(
            super::import_package(encoded, bytes_root.path(), PreparedInstall::activate,)
                .unwrap()
                .version,
            "2"
        );
    }
    let root = tempfile::tempdir().unwrap();
    assert!(!import(&package("1"), root.path()).unwrap().replaced);
}

#[test]
fn supports_both_manifests_flat_and_single_wrapper_and_priority() {
    for wrapper in ["", "archive-root/"] {
        for manifest_file in ["plugin.json", "manifest.json"] {
            let root = tempfile::tempdir().unwrap();
            let name = format!("{wrapper}{manifest_file}");
            let entry = format!("{wrapper}main.py");
            let mut entries = vec![
                (name.as_str(), manifest("demo", "1")),
                (entry.as_str(), b"entry".to_vec()),
            ];
            if !wrapper.is_empty() {
                entries.push((wrapper, Vec::new()));
            }
            let borrowed: Vec<_> = entries
                .iter()
                .map(|(name, bytes)| (*name, bytes.as_slice()))
                .collect();
            import(&zip(&borrowed), root.path()).unwrap();
            assert!(root.path().join("demo").join(manifest_file).is_file());
            assert!(!root.path().join("demo/archive-root").exists());
        }
    }
    let root = tempfile::tempdir().unwrap();
    let bytes = zip(&[
        ("plugin.json", &manifest("demo", "1")),
        ("manifest.json", &manifest("ignored", "9")),
        ("main.py", b"entry"),
    ]);
    assert_eq!(import(&bytes, root.path()).unwrap().name, "demo");
    let bad_preferred = zip(&[
        ("plugin.json", b"bad"),
        ("manifest.json", &manifest("demo", "1")),
        ("main.py", b"entry"),
    ]);
    assert!(import(&bad_preferred, root.path()).is_err());
    assert!(root.path().join("demo/main.py").is_file());
}

#[test]
fn overwrite_replaces_whole_tree_retains_backup_in_the_single_root() {
    let root = tempfile::tempdir().unwrap();
    import(&package("1"), root.path()).unwrap();
    fs::write(root.path().join("demo/old-only.py"), b"keep in backup").unwrap();
    let result = import_with_reload(request(&package("2")), root.path(), || {}).unwrap();
    assert!(result.replaced);
    assert_eq!(result.version, "2");
    assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"2");
    assert!(!root.path().join("demo/old-only.py").exists());
    let transaction = fs::read_dir(root.path().join(".timepin-installs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read(transaction.join("backup/main.py")).unwrap(), b"1");
    assert_eq!(
        fs::read(transaction.join("backup/old-only.py")).unwrap(),
        b"keep in backup"
    );
    assert!(!transaction.join("staging").exists());
    let discoverable: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .flatten()
        .filter_map(|e| PluginManifest::load(&e.path()).ok())
        .map(|m| m.name)
        .collect();
    assert_eq!(discoverable, vec!["demo"]);
}

#[test]
fn local_import_cannot_replace_the_bundled_recognition_service() {
    for name in ["recognition", "ReCoGnItIoN"] {
        let root = tempfile::tempdir().unwrap();
        let service = root.path().join("recognition");
        fs::create_dir(&service).unwrap();
        fs::write(service.join("main.py"), b"bundled service").unwrap();
        let bytes = zip(&[
            ("plugin.json", &manifest(name, "1")),
            ("main.py", b"replacement"),
        ]);
        let callbacks = AtomicUsize::new(0);
        let result = super::import_package(request(&bytes), root.path(), |install| {
            callbacks.fetch_add(1, Ordering::SeqCst);
            install.activate()
        });
        assert!(result.unwrap_err().contains("内置识别服务保留目录"));
        assert_eq!(callbacks.load(Ordering::SeqCst), 0);
        assert_eq!(
            fs::read(service.join("main.py")).unwrap(),
            b"bundled service"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn same_root_manifest_alias_is_rejected_but_target_overwrite_is_allowed() {
    let root = tempfile::tempdir().unwrap();
    let package_bytes = package("2");
    let alias = root.path().join("alias");
    import(&package("1"), root.path()).unwrap();
    fs::rename(root.path().join("demo"), &alias).unwrap();
    // A different direct child already owns manifest.name: do not install.
    let error = super::import_package(request(&package_bytes), root.path(), |_| {
        panic!("alias reached lifecycle callback")
    })
    .unwrap_err();
    assert!(error.contains("同名插件"));
    assert!(error.contains(&alias.display().to_string()));
    assert_eq!(fs::read(alias.join("main.py")).unwrap(), b"1");
    assert!(!root.path().join("demo").exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    // Normal overwrite of the install target itself remains supported.
    fs::rename(&alias, root.path().join("demo")).unwrap();
    let result = super::import_package(
        request(&package_bytes),
        root.path(),
        PreparedInstall::activate,
    )
    .unwrap();
    assert!(result.replaced);
    assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"2");
}

#[test]
fn validates_and_extracts_before_stop_then_switches_before_reload() {
    let root = tempfile::tempdir().unwrap();
    import(&package("old"), root.path()).unwrap();
    for bytes in [b"not zip".to_vec(), zip(&[("plugin.json", b"bad")])] {
        assert!(super::import_package(request(&bytes), root.path(), |_| {
            Err::<ImportPluginResult, _>("invalid package reached lifecycle callback".to_owned())
        })
        .is_err());
    }
    let mut events = Vec::new();
    super::import_package(request(&package("new")), root.path(), |install| {
        assert_eq!(fs::read(install.root.join("main.py")).unwrap(), b"new");
        assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"old");
        assert!(
            INSTALL_LOCK.try_lock().is_err(),
            "install lock includes stop/reload"
        );
        events.push("stop");
        let result = install.activate()?;
        events.push("switch");
        assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"new");
        events.push("reload");
        Ok(result)
    })
    .unwrap();
    assert_eq!(events, ["stop", "switch", "reload"]);
}

#[test]
fn dropping_a_validated_stage_preserves_install_without_backup() {
    let root = tempfile::tempdir().unwrap();
    import(&package("old"), root.path()).unwrap();
    assert!(
        super::import_package(request(&package("new")), root.path(), |_| {
            Err("lifecycle transition rejected".to_owned())
        })
        .is_err()
    );
    assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"old");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn imports_create_the_single_root_and_block_invalid_roots_without_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("plugins");
    assert!(!root.exists());
    import(&package("1"), &root).unwrap();
    assert_eq!(fs::read(root.join("demo/main.py")).unwrap(), b"1");
    let blocked = directory.path().join("blocked");
    fs::write(&blocked, b"preserve").unwrap();
    for bad in [
        PathBuf::from("relative"),
        PathBuf::new(),
        blocked.clone(),
        blocked.join("plugins"),
    ] {
        assert!(
            import_with_reload(request(&package("2")), &bad, || panic!("must not reload")).is_err()
        );
    }
    assert_eq!(fs::read(blocked).unwrap(), b"preserve");
    assert_eq!(fs::read(root.join("demo/main.py")).unwrap(), b"1");
}

#[test]
fn case_variant_targets_are_reused_or_rejected_according_to_filesystem_identity() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("demo");
    let second = root.path().join("DEMO");
    fs::create_dir(&first).unwrap();
    fs::write(first.join("plugin.json"), manifest("other", "1")).unwrap();
    match fs::create_dir(&second) {
        Ok(()) => {
            // Case-sensitive filesystems can hold two colliding target names.
            let error = import_with_reload(request(&package("2")), root.path(), || {
                panic!("conflicting targets must not reload")
            })
            .unwrap_err();
            assert!(error.contains("大小写冲突"));
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Windows/default macOS: both casings are the same install target,
            // not two competing manifest aliases. Existing targets may update.
            assert_eq!(install_target(root.path(), "DEMO").unwrap(), first);
            let result = import(&package("2"), root.path()).unwrap();
            assert!(result.replaced);
            assert_eq!(fs::read(first.join("main.py")).unwrap(), b"2");
            assert_eq!(fs::read(second.join("main.py")).unwrap(), b"2");
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
        }
        Err(error) => panic!("cannot create case-variant test directory: {error}"),
    }
}

#[test]
fn invalid_schema_source_or_extension_does_not_create_installs() {
    let root = tempfile::tempdir().unwrap();
    let mut unsupported = request(&package("1"));
    unsupported.schema_version = "1.0.0".to_owned();
    assert!(
        import_with_reload(unsupported, root.path(), || panic!("must not reload"))
            .unwrap_err()
            .contains("schemaVersion")
    );
    for (name, base64) in [
        ("plugin.txt", ""),
        ("../plugin.zip", ""),
        ("plugin.zip", "not base64"),
        ("C:plugin.zip", ""),
    ] {
        let request = ImportPluginRequest {
            source: ImportPluginSource::Bytes(EncodedPackage {
                base64: base64.into(),
                file_name: name.into(),
            }),
            schema_version: "1.0".into(),
        };
        assert!(import_with_reload(request, root.path(), || panic!("must not reload")).is_err());
    }
    assert!(read_source(ImportPluginSource::Path(
        root.path()
            .join("missing.zip")
            .to_string_lossy()
            .into_owned()
    ))
    .is_err());
    assert!(serde_json::from_value::<ImportPluginRequest>(
        serde_json::json!({"source":{"base64":"", "file_name":"x.zip"}})
    )
    .is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn unsafe_or_broken_packages_preserve_old_install_and_do_not_reload() {
    let root = tempfile::tempdir().unwrap();
    import(&package("old"), root.path()).unwrap();
    let valid = manifest("demo", "new");
    let invalid_manifest = |field: &str, value: &str| {
        let mut json: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        json[field] = value.into();
        serde_json::to_vec(&json).unwrap()
    };
    let mut bad = vec![
        b"not a zip".to_vec(),
        zip(&[]),
        zip(&[("plugin.json", b"invalid json"), ("main.py", b"new")]),
        zip(&[("plugin.json", &valid)]),
        zip(&[
            ("a/plugin.json", &valid),
            ("a/main.py", b"new"),
            ("extra.txt", b"extra"),
        ]),
        zip(&[("a/plugin.json", &valid), ("b/main.py", b"new")]),
        zip(&[
            ("nested/deeper/plugin.json", &valid),
            ("nested/deeper/main.py", b"new"),
        ]),
    ];
    for (field, value) in [
        ("name", "../evil"),
        ("name", "CON"),
        ("name", "LPT1"),
        ("protocolVersion", "2.0.0"),
        ("entry", "../main.py"),
        ("entry", "C:/evil.py"),
        ("entry", "sub\\main.py"),
        ("entry", "/main.py"),
        ("entry", "main.py/"),
        ("entry", "folder"),
    ] {
        bad.push(zip(&[
            ("plugin.json", &invalid_manifest(field, value)),
            ("main.py", b"new"),
            ("folder/", b""),
        ]));
    }
    for bytes in bad {
        assert!(
            import_with_reload(request(&bytes), root.path(), || panic!("must not reload")).is_err()
        );
        assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"old");
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            1,
            "no stage/backup on validation error"
        );
    }
}

#[test]
fn unsafe_archive_paths_aliases_duplicates_and_links_are_rejected() {
    for path in [
        "../evil",
        "/evil",
        "C:/evil",
        "C:evil",
        "\\evil",
        "a\\evil",
        "a//evil",
        "a/./evil",
        "a/../evil",
        "a///",
        "NUL",
        "con.txt",
        "con .txt",
        "PRN",
        "AUX.py",
        "COM1.txt",
        "lpt9",
        "COM¹.log",
        "folder./evil",
        "folder /evil",
        "a/name.",
        "a/name ",
        "a/b:stream",
        "a/evil?",
        "a/evil\u{1}",
    ] {
        let root = tempfile::tempdir().unwrap();
        assert!(
            import(
                &zip(&[
                    ("plugin.json", &manifest("demo", "1")),
                    ("main.py", b"entry"),
                    (path, b"bad")
                ]),
                root.path()
            )
            .is_err(),
            "accepted {path:?}"
        );
        assert!(!root.path().join("demo").exists());
    }
    for entries in [
        vec![("main.py", &b"one"[..]), ("MAIN.py", &b"two"[..])],
        vec![("sub/a", &b"one"[..]), ("Sub/b", &b"two"[..])],
        vec![("Σ.txt", &b"one"[..]), ("ς.txt", &b"two"[..])],
        vec![("sub", &b"one"[..]), ("sub/a", &b"two"[..])],
        vec![("a/b", &b"one"[..]), ("a", &b"two"[..])],
        vec![("a", &b"one"[..]), ("a", &b"two"[..])],
    ] {
        let root = tempfile::tempdir().unwrap();
        assert!(extract_zip(
            Cursor::new(zip(&entries)),
            root.path(),
            Layout::Preserve,
            EXTRACTION_LIMITS
        )
        .is_err());
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("link", "../outside", FileOptions::default())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    assert!(extract_zip(
        Cursor::new(writer.finish().unwrap().into_inner()),
        root.path(),
        Layout::Preserve,
        EXTRACTION_LIMITS
    )
    .is_err());
}

#[test]
fn compressed_encoded_and_extracted_limits_are_enforced() {
    assert!(check_archive_length(MAX_ARCHIVE_BYTES).is_ok());
    assert!(check_archive_length(MAX_ARCHIVE_BYTES + 1).is_err());
    assert!(check_base64_length(MAX_BASE64_BYTES).is_ok());
    assert!(check_base64_length(MAX_BASE64_BYTES + 1).is_err());
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("large.zip");
    fs::File::create(&file)
        .unwrap()
        .set_len(MAX_ARCHIVE_BYTES + 1)
        .unwrap();
    assert!(read_source(ImportPluginSource::Path(
        file.to_string_lossy().into_owned()
    ))
    .unwrap_err()
    .contains("128 MiB"));
    let bytes = zip(&[("a", b"123"), ("b", b"456")]);
    for limits in [
        ExtractionLimits {
            entries: 1,
            entry_bytes: 10,
            total_bytes: 10,
        },
        ExtractionLimits {
            entries: 10,
            entry_bytes: 2,
            total_bytes: 10,
        },
        ExtractionLimits {
            entries: 10,
            entry_bytes: 10,
            total_bytes: 5,
        },
    ] {
        let root = tempfile::tempdir().unwrap();
        assert!(extract_zip(Cursor::new(&bytes), root.path(), Layout::Preserve, limits).is_err());
    }
    let root = tempfile::tempdir().unwrap();
    extract_zip(
        Cursor::new(&bytes),
        root.path(),
        Layout::Preserve,
        ExtractionLimits {
            entries: 2,
            entry_bytes: 3,
            total_bytes: 6,
        },
    )
    .unwrap();
    assert_eq!(fs::read(root.path().join("b")).unwrap(), b"456");
}

#[test]
fn corrupt_crc_and_truncated_archives_preserve_old_install() {
    let root = tempfile::tempdir().unwrap();
    import(&package("old"), root.path()).unwrap();
    let mut bytes = package("new");
    // Corrupt a central-directory CRC (payload may be compressed).
    let position = bytes
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap();
    bytes[position + 16] ^= 1;
    assert!(import(&bytes, root.path()).is_err());
    bytes.truncate(bytes.len() / 2);
    assert!(import(&bytes, root.path()).is_err());
    assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"old");
}

#[test]
fn failed_switch_rolls_back_and_non_directory_target_is_untouched() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("demo");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("old"), b"original").unwrap();
    let backup = root.path().join("backup");
    assert!(switch_install(&root.path().join("missing-stage"), &target, &backup).is_err());
    assert_eq!(fs::read(target.join("old")).unwrap(), b"original");
    assert!(!backup.exists());
    let blocked = tempfile::tempdir().unwrap();
    fs::write(blocked.path().join("demo"), b"not a directory").unwrap();
    assert!(import(&package("1"), blocked.path()).is_err());
    assert_eq!(
        fs::read(blocked.path().join("demo")).unwrap(),
        b"not a directory"
    );
}

#[test]
fn same_process_imports_are_serialized_through_reload() {
    let root = tempfile::tempdir().unwrap();
    let active = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                import_with_reload(request(&package("1")), root.path(), || {
                    assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                    std::thread::yield_now();
                    assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
                })
                .unwrap();
            });
        }
    });
    assert_eq!(fs::read(root.path().join("demo/main.py")).unwrap(), b"1");
}

#[cfg(unix)]
#[test]
fn rejects_symlink_target_root_and_workspace_without_touching_referent() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), b"original").unwrap();
    symlink(outside.path(), root.path().join("demo")).unwrap();
    assert!(import(&package("1"), root.path()).is_err());
    fs::remove_file(root.path().join("demo")).unwrap();
    symlink(outside.path(), root.path().join(".timepin-installs")).unwrap();
    assert!(import(&package("1"), root.path()).is_err());
    assert!(import(&package("1"), &root.path().join(".timepin-installs")).is_err());
    assert_eq!(fs::read(outside.path().join("keep")).unwrap(), b"original");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn rejects_windows_junction_target_root_and_workspace_without_touching_referent() {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), b"original").unwrap();
    for name in ["demo", ".timepin-installs"] {
        let junction = root.path().join(name);
        let status = Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(status.status.success(), "junction creation failed");
        assert!(is_link(&fs::symlink_metadata(&junction).unwrap()));
        assert!(import(&package("1"), root.path()).is_err());
        assert!(import(&package("1"), &junction).is_err());
        fs::remove_dir(&junction).unwrap();
    }
    assert_eq!(fs::read(outside.path().join("keep")).unwrap(), b"original");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}
