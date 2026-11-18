# Changelog

## Unreleased

### Changed

- **Breaking — plugin directory (TPIN-27):** plugin discovery, reload, local `.tppkg`/`.zip` imports and GitHub installation now share `<exe_dir>/plugins`. The directory is created at startup; custom plugin directory settings and the import setup prerequisite have been removed.
- Old application-data, custom, bundled-resource and repository-development plugin roots are no longer scanned. Existing files are not moved or deleted automatically. Back up and copy complete plugin folders and shared assets to the new directory, resolve same-name conflicts, then reload. See the [migration guide](docs/plugin-import.md#旧版本迁移破坏性变更) for instructions and platform limitations.
- Development builds use the same executable-adjacent directory as packaged builds. Copy the required repository plugins beside the actual debug/release executable; changing the working directory does not change the plugin root.

### Fixed

- Reserve the `recognition` directory, case-insensitively, in local and GitHub installers so ordinary plugins cannot replace the bundled recognition sidecar in the shared root.

Version fields remain unchanged in feature PRs. A release PR from `beta` to `main` selects the release version and updates all three version fields together.
