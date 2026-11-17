#!/usr/bin/env python3
"""Release contract regression tests. Run: python -I scripts/test_release.py."""
from __future__ import annotations

import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("release", ROOT / "scripts" / "release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def key_fixture(comment="untrusted comment: tauri"):
    # Tauri/minisign public key text, base64-wrapped as updater config expects.
    raw = b"Ed" + bytes(40)
    text = f"{comment}\n{base64.b64encode(raw).decode()}\n"
    return base64.b64encode(text.encode()).decode()


def signature_fixture(trusted="trusted comment: timestamp:1"):
    text = ("untrusted comment: signature\n"
            + base64.b64encode(b"Ed" + bytes(72)).decode() + "\n"
            + trusted + "\n"
            + base64.b64encode(bytes(64)).decode() + "\n")
    return base64.b64encode(text.encode()).decode()


class Fixture:
    def __init__(self, case):
        self.case = case
        self.path = Path(tempfile.mkdtemp(prefix="timepin-release-test-"))
        (self.path / "src-tauri").mkdir()
        (self.path / "package.json").write_text('{"version":"0.0.6"}')
        config = {
            "productName": "TimePin", "identifier": "com.timepin.app",
            "version": "0.0.6",
            "app": {"windows": [{"label": "main"}]},
            "plugins": {"updater": {
                "pubkey": "", "endpoints": [release.ENDPOINT],
                "windows": {"installMode": "passive"}}},
            "bundle": {"createUpdaterArtifacts": False},
        }
        (self.path / "src-tauri/tauri.conf.json").write_text(json.dumps(config))
        (self.path / "src-tauri/Cargo.toml").write_text('[package]\nversion="0.0.6"\n')

    def close(self):
        shutil.rmtree(self.path)


def fixture(test):
    f = Fixture(test)
    test.addCleanup(f.close)
    return f.path


def staged(platform, root):
    directory = root / ("bundle-" + platform)
    directory.mkdir(parents=True)
    for _, suffix, keys in release.ASSETS[platform]:
        name = release.asset_name("0.0.6", platform, suffix)
        (directory / name).write_bytes((platform + suffix).encode())
        if keys:
            (directory / (name + ".sig")).write_text(signature_fixture())
    return directory


class ContractTests(unittest.TestCase):
    def test_version_identity_and_endpoint_guard(self):
        root = fixture(self)
        self.assertEqual(release.validate_config(root), "0.0.6")
        config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
        config["identifier"] = "com.other.app"
        (root / "src-tauri/tauri.conf.json").write_text(json.dumps(config))
        with self.assertRaisesRegex(ValueError, "Installer identity"):
            release.validate_config(root)

    def test_main_window_label_contract_exists(self):
        root = fixture(self)
        config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
        self.assertEqual(config["app"]["windows"][0]["label"], "main")

    def test_public_key_must_be_minisign_text_not_a_path(self):
        self.assertEqual(release.public_key_text(key_fixture()).splitlines()[1],
                         base64.b64encode(b"Ed" + bytes(40)).decode())
        for bad in ("", "updater.pub", base64.b64encode(b"not-a-key").decode()):
            with self.assertRaises(ValueError):
                release.public_key_text(bad)

    def test_override_enables_signed_artifacts_without_identity_change(self):
        override = release.release_override(key_fixture())
        self.assertEqual(override["bundle"], {"createUpdaterArtifacts": True})
        self.assertEqual(override["plugins"]["updater"]["pubkey"], key_fixture())
        self.assertNotIn("identifier", override)
        self.assertNotIn("version", override)

    def test_stage_rejects_missing_and_duplicate_assets(self):
        root = fixture(self)
        bundle = Path(tempfile.mkdtemp(prefix="timepin-bundle-"))
        self.addCleanup(shutil.rmtree, bundle)
        with self.assertRaisesRegex(ValueError, "Expected exactly one"):
            release.stage(root, bundle, "windows-x86_64", root / "out")
        target = bundle / "nsis"
        target.mkdir()
        (target / "a-setup.exe").write_bytes(b"a")
        (target / "a-setup.exe.sig").write_text(signature_fixture())
        with self.assertRaisesRegex(ValueError, "Expected exactly one"):
            release.stage(root, bundle, "windows-x86_64", root / "out")

    def test_assemble_never_writes_partial_latest_json(self):
        root = fixture(self)
        inputs = Path(tempfile.mkdtemp(prefix="timepin-input-"))
        self.addCleanup(shutil.rmtree, inputs)
        for platform in release.ASSETS:
            staged(platform, inputs)
        # Missing one extra artifact (deb) must stop before latest.json exists.
        (inputs / "bundle-linux-x86_64" / release.asset_name("0.0.6", "linux-x86_64", ".deb")).unlink()
        with mock.patch.object(release, "verify_signature"):
            with self.assertRaises(ValueError):
                release.assemble(root, inputs, root / "ready", key_fixture())
        self.assertFalse((root / "ready/latest.json").exists())

    def test_missing_minisign_fails_closed_before_feed(self):
        root = fixture(self)
        inputs = Path(tempfile.mkdtemp(prefix="timepin-input-"))
        self.addCleanup(shutil.rmtree, inputs)
        for platform in release.ASSETS:
            staged(platform, inputs)
        with mock.patch.object(release.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ValueError, "minisign is required"):
                release.assemble(root, inputs, root / "ready", key_fixture())
        self.assertFalse((root / "ready/latest.json").exists())

    def test_supported_runner_matrix_is_explicit(self):
        self.assertEqual({item["os"] for item in release.TARGETS},
                         {"windows-2022", "ubuntu-22.04", "macos-15", "macos-15-intel"})
        self.assertEqual(len({item["target"] for item in release.TARGETS}), 4)


@unittest.skipUnless(shutil.which("minisign"), "minisign verifier not installed")
class CryptographicVerificationTests(unittest.TestCase):
    def test_verification_rejects_tampered_artifact(self):
        directory = Path(tempfile.mkdtemp(prefix="timepin-minisign-"))
        self.addCleanup(shutil.rmtree, directory)
        keys = subprocess.run(["minisign", "-G", "-p", str(directory / "key.pub"),
                               "-s", str(directory / "key"), "-W"], check=True)
        artifact = directory / "artifact.bin"
        artifact.write_bytes(b"release payload")
        subprocess.run(["minisign", "-S", "-m", str(artifact), "-s", str(directory / "key")],
                       check=True, stdout=subprocess.DEVNULL)
        signature = directory / "artifact.bin.minisig"
        signature.rename(directory / "artifact.bin.sig")
        public = base64.b64encode((directory / "key.pub").read_bytes()).decode()
        release.verify_signature(artifact, directory / "artifact.bin.sig",
                                 release.public_key_text(public))
        artifact.write_bytes(b"tampered payload")
        with self.assertRaises(ValueError):
            release.verify_signature(artifact, directory / "artifact.bin.sig",
                                     release.public_key_text(public))


if __name__ == "__main__":
    unittest.main(verbosity=2)
