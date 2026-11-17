#!/usr/bin/env python3
"""Release contract, deterministic staging and complete updater feed (Python 3.11+).

No signing keys are generated or stored. Signing is performed only by Tauri's
bundler. Use minisign in the aggregation job to verify each updater artifact.
"""
from __future__ import annotations

import argparse
import base64
import binascii
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import quote

REPOSITORY = "H2SYJ/TimePin"
ENDPOINT = f"https://github.com/{REPOSITORY}/releases/latest/download/latest.json"
# Explicit CPU targets: never infer Intel/ARM from a mutable *-latest runner.
TARGETS = [
    {"platform": "windows-x86_64", "os": "windows-2022", "target": "x86_64-pc-windows-msvc", "bundles": "msi,nsis"},
    {"platform": "linux-x86_64", "os": "ubuntu-22.04", "target": "x86_64-unknown-linux-gnu", "bundles": "appimage,deb"},
    {"platform": "darwin-aarch64", "os": "macos-15", "target": "aarch64-apple-darwin", "bundles": "app,dmg"},
    {"platform": "darwin-x86_64", "os": "macos-15-intel", "target": "x86_64-apple-darwin", "bundles": "app,dmg"},
]
# (source suffix, published suffix, updater manifest keys). Empty keys = manual installer.
ASSETS = {
    "windows-x86_64": [
        (".msi", ".msi", ["windows-x86_64", "windows-x86_64-msi"]),
        ("-setup.exe", "-setup.exe", ["windows-x86_64-nsis"]),
    ],
    "linux-x86_64": [(".AppImage", ".AppImage", ["linux-x86_64"]), (".deb", ".deb", [])],
    "darwin-aarch64": [(".app.tar.gz", ".app.tar.gz", ["darwin-aarch64"]), (".dmg", ".dmg", [])],
    "darwin-x86_64": [(".app.tar.gz", ".app.tar.gz", ["darwin-x86_64"]), (".dmg", ".dmg", [])],
}
STABLE_VERSION = re.compile(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\Z")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def version_of(root: Path) -> str:
    package = load_json(root / "package.json")
    config = load_json(root / "src-tauri/tauri.conf.json")
    cargo = tomllib.loads((root / "src-tauri/Cargo.toml").read_text(encoding="utf-8"))
    versions = [package["version"], config["version"], cargo["package"]["version"]]
    require(len(set(versions)) == 1, "Version mismatch: package.json, Cargo.toml and tauri.conf.json must agree")
    require(bool(STABLE_VERSION.fullmatch(versions[0])), "Stable releases require an X.Y.Z version (no prerelease/build suffix)")
    return versions[0]


def validate_config(root: Path) -> str:
    version = version_of(root)
    config = load_json(root / "src-tauri/tauri.conf.json")
    require(config["identifier"] == "com.timepin.app" and config["productName"] == "TimePin", "Installer identity must remain com.timepin.app / TimePin")
    windows = config.get("app", {}).get("windows") or []
    require(windows and windows[0].get("label") == "main", "Main window must keep label 'main' for updater ACL scoping")
    updater = config["plugins"]["updater"]
    require(updater["endpoints"] == [ENDPOINT], "Updater endpoint must use the complete stable latest.json feed")
    require(not updater.get("dangerousInsecureTransportProtocol", False), "Insecure updater transport is forbidden")
    require(config["bundle"].get("createUpdaterArtifacts") is False, "Development builds must not require release signing credentials")
    return version


def public_key_text(value: str) -> str:
    try:
        text = base64.b64decode(value.strip(), validate=True).decode("utf-8")
        lines = text.splitlines()
        raw = base64.b64decode(lines[1], validate=True)
        require(lines[0].startswith("untrusted comment:") and len(raw) == 42 and raw[:2] == b"Ed", "Invalid updater public key")
        return text
    except (binascii.Error, UnicodeError, IndexError) as error:
        raise ValueError("TAURI_UPDATER_PUBLIC_KEY must contain the Tauri base64 public key, not a path") from error


def release_override(public_key: str) -> dict:
    public_key_text(public_key)
    return {"plugins": {"updater": {"pubkey": public_key.strip()}}, "bundle": {"createUpdaterArtifacts": True}}


def preflight(root: Path, override: Path | None, github_output: Path | None, release: bool) -> None:
    version = validate_config(root)
    if release:
        require(os.environ.get("GITHUB_REF") == "refs/heads/main", "Releases may only be built from main")
        require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "Releases may only be built in H2SYJ/TimePin")
        for name in ("TAURI_UPDATER_PUBLIC_KEY", "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"):
            require(bool(os.environ.get(name, "").strip()), f"Missing repository variable/secret: {name}")
        public_key_text(os.environ["TAURI_UPDATER_PUBLIC_KEY"])
    if override:
        # The override exists only to inject a valid public key; it must never
        # silently write an unconfigured key or private material.
        write_json(override, release_override(os.environ.get("TAURI_UPDATER_PUBLIC_KEY", "")))
    if github_output:
        with github_output.open("a", encoding="utf-8") as output:
            output.write(f"version={version}\n")
            output.write("matrix=" + json.dumps({"include": TARGETS}, separators=(",", ":")) + "\n")
    print(f"Release configuration valid: v{version}")


def asset_name(version: str, platform: str, suffix: str) -> str:
    return f"TimePin_{version}_{platform}{suffix}"


def signature_text(path: Path) -> str:
    require(path.is_file() and not path.is_symlink(), f"Missing signature: {path.name}")
    signature = path.read_text(encoding="utf-8").strip()
    try:
        decoded = base64.b64decode(signature, validate=True).decode("utf-8")
    except (binascii.Error, UnicodeError) as error:
        raise ValueError(f"Invalid signature encoding: {path.name}") from error
    require(decoded.startswith("untrusted comment:") and "\ntrusted comment:" in decoded, f"Invalid minisign signature: {path.name}")
    return signature


def stage(root: Path, bundle: Path, platform: str, output: Path) -> None:
    version = version_of(root)
    require(platform in ASSETS, f"Unsupported platform: {platform}")
    require(not output.exists(), "Stage output must be a new directory")
    candidates = list(bundle.rglob("*"))
    selected = []
    for suffix, published_suffix, keys in ASSETS[platform]:
        matches = [path for path in candidates if path.is_file() and path.name.endswith(suffix)]
        require(len(matches) == 1, f"Expected exactly one {suffix} for {platform}; found {len(matches)}")
        source = matches[0]
        require(not source.is_symlink() and source.stat().st_size > 0, f"Invalid release asset: {source.name}")
        if keys:
            signature_text(Path(str(source) + ".sig"))
        selected.append((source, asset_name(version, platform, published_suffix), keys))
    output.mkdir(parents=True)
    for source, name, keys in selected:
        shutil.copyfile(source, output / name)
        if keys:
            shutil.copyfile(Path(str(source) + ".sig"), output / (name + ".sig"))
    print(f"Staged {platform}: {len(selected)} installers/update bundles")


def verify_signature(artifact: Path, signature: Path, key_text: str) -> None:
    """Tauri wraps standard minisign key/signature text in base64."""
    decoded = base64.b64decode(signature_text(signature), validate=True)
    verifier = shutil.which("minisign")
    require(bool(verifier), "minisign is required to verify updater signatures")
    with tempfile.TemporaryDirectory(prefix="timepin-verify-") as temporary:
        directory = Path(temporary)
        key_path, signature_path = directory / "key.pub", directory / "update.minisig"
        key_path.write_text(key_text, encoding="utf-8")
        signature_path.write_bytes(decoded)
        result = subprocess.run([verifier, "-Vm", str(artifact.resolve()), "-p", str(key_path), "-x", str(signature_path)], capture_output=True, text=True, check=False)
        require(result.returncode == 0, f"Updater signature verification failed: {artifact.name}")


def assemble(root: Path, inputs: Path, output: Path, public_key: str) -> None:
    version = validate_config(root)
    key_text = public_key_text(public_key)
    require(not output.exists(), "Assembly output must be a new directory")
    require({path.name for path in inputs.iterdir()} == {"bundle-" + item["platform"] for item in TARGETS}, "All four platform artifacts are required; no partial latest.json allowed")
    pending, platforms = [], {}
    for platform, specs in ASSETS.items():
        directory = inputs / ("bundle-" + platform)
        require(directory.is_dir() and not directory.is_symlink(), f"Invalid platform directory: {platform}")
        expected = set()
        for _, suffix, keys in specs:
            name = asset_name(version, platform, suffix)
            expected.add(name)
            artifact = directory / name
            require(artifact.is_file() and not artifact.is_symlink() and artifact.stat().st_size > 0, f"Missing/empty artifact: {name}")
            pending.append(artifact)
            if keys:
                expected.add(name + ".sig")
                signature = directory / (name + ".sig")
                verify_signature(artifact, signature, key_text)
                pending.append(signature)
                record = {"signature": signature_text(signature), "url": f"https://github.com/{REPOSITORY}/releases/download/v{version}/{quote(name)}"}
                platforms.update({key: record.copy() for key in keys})
        require({path.name for path in directory.iterdir()} == expected, f"Unexpected or missing files for {platform}")
    # Do not expose even a local feed until every required artifact verifies.
    output.mkdir(parents=True)
    for source in pending:
        shutil.copyfile(source, output / source.name)
    write_json(output / "latest.json", {
        "version": version,
        "notes": f"TimePin v{version}. See the release notes on GitHub.",
        "pub_date": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "platforms": platforms,
    })
    sums = []
    for path in sorted(output.iterdir()):
        with path.open("rb") as file:
            digest = hashlib.file_digest(file, "sha256").hexdigest()
        sums.append(f"{digest}  {path.name}")
    (output / "SHA256SUMS").write_text("\n".join(sums) + "\n", encoding="utf-8")
    print(f"Validated v{version}: {len(pending)} assets/signatures, {len(platforms)} updater targets")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("check")
    check.add_argument("--release", action="store_true")
    check.add_argument("--override", type=Path)
    check.add_argument("--github-output", type=Path)
    staging = commands.add_parser("stage")
    staging.add_argument("--bundle", type=Path, required=True)
    staging.add_argument("--platform", required=True, choices=list(ASSETS))
    staging.add_argument("--output", type=Path, required=True)
    aggregation = commands.add_parser("assemble")
    aggregation.add_argument("--inputs", type=Path, required=True)
    aggregation.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "check":
        preflight(args.root, args.override, args.github_output, args.release)
    elif args.command == "stage":
        stage(args.root, args.bundle, args.platform, args.output)
    elif args.command == "assemble":
        assemble(args.root, args.inputs, args.output, os.environ.get("TAURI_UPDATER_PUBLIC_KEY", ""))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        print(f"Release validation failed: {error}", file=sys.stderr)
        sys.exit(1)
