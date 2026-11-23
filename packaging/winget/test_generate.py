#!/usr/bin/env python3
"""Tests for the WinGet manifest generator.

    python3 -m unittest discover -s packaging/winget -v

PyYAML and jsonschema are optional. Set WINGET_SCHEMA_DIR to the official
1.6.0 schema files to enable the schema test.
"""

import contextlib
import hashlib
import io
import json
import os
import shutil
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None

try:
    import jsonschema
except ImportError:
    jsonschema = None

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import generate  # noqa: E402

VERSION = "0.4.0"
CLI_ZIP = f"hp-charge-limit-rs-v{VERSION}-x86_64-pc-windows-msvc.zip"
CLI_NESTED = (
    f"hp-charge-limit-rs-v{VERSION}-x86_64-pc-windows-msvc/hp-charge-limit-rs.exe"
)
DESKTOP_EXE = f"hp-charge-limit-desktop-v{VERSION}-x86_64-setup.exe"
CLI_DIR = Path("manifests/k/kralonur/HPChargeLimit") / VERSION
DESKTOP_DIR = Path("manifests/k/kralonur/HPChargeLimit/Desktop") / VERSION
EXPECTED = {
    CLI_DIR / "kralonur.HPChargeLimit.yaml",
    CLI_DIR / "kralonur.HPChargeLimit.installer.yaml",
    CLI_DIR / "kralonur.HPChargeLimit.locale.en-US.yaml",
    DESKTOP_DIR / "kralonur.HPChargeLimit.Desktop.yaml",
    DESKTOP_DIR / "kralonur.HPChargeLimit.Desktop.installer.yaml",
    DESKTOP_DIR / "kralonur.HPChargeLimit.Desktop.locale.en-US.yaml",
}


class GeneratorTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="winget-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.dist = self.tmp / "dist"
        self.dist.mkdir()
        self.out = self.tmp / "out"
        self.cli_zip = self.dist / CLI_ZIP
        self.desktop_exe = self.dist / DESKTOP_EXE
        self._write_cli_zip()
        self.desktop_exe.write_bytes(b"desktop dummy bytes")

    def _write_cli_zip(self, nested=CLI_NESTED, data=b"cli dummy bytes"):
        with zipfile.ZipFile(self.cli_zip, "w") as archive:
            archive.writestr(nested, data)

    def _run(self, *, version=VERSION, repository=generate.DEFAULT_REPOSITORY):
        argv = [
            f"--version={version}",
            f"--artifacts={self.dist}",
            f"--output={self.out}",
            f"--repository={repository}",
        ]
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = generate.main(argv)
        return code, stdout.getvalue(), stderr.getvalue()

    def _read(self, relative):
        return (self.out / relative).read_text(encoding="utf-8")

    def _assert_no_output(self):
        if self.out.exists():
            self.assertEqual(list(self.out.rglob("*.yaml")), [])

    def _reject(self, *, version=VERSION, repository=generate.DEFAULT_REPOSITORY, text=None):
        code, _, stderr = self._run(version=version, repository=repository)
        self.assertNotEqual(code, 0)
        if text:
            self.assertIn(text, stderr)
        self._assert_no_output()
        return stderr

    def test_six_files_and_metadata(self):
        code, _, stderr = self._run()
        self.assertEqual(code, 0, stderr)
        actual = {
            path.relative_to(self.out)
            for path in self.out.rglob("*")
            if path.is_file()
        }
        self.assertEqual(actual, EXPECTED)

        cli_hash = hashlib.sha256(self.cli_zip.read_bytes()).hexdigest().upper()
        desktop_hash = (
            hashlib.sha256(self.desktop_exe.read_bytes()).hexdigest().upper()
        )
        cli = self._read(CLI_DIR / "kralonur.HPChargeLimit.installer.yaml")
        for expected in (
            f'InstallerSha256: "{cli_hash}"',
            f'RelativeFilePath: "{CLI_NESTED}"',
            'PortableCommandAlias: "hp-charge-limit-rs"',
            'Scope: "user"',
            "https://github.com/kralonur/hp-charge-limit-rs/releases/download/"
            f"v{VERSION}/{CLI_ZIP}",
        ):
            self.assertIn(expected, cli)

        desktop = self._read(
            DESKTOP_DIR / "kralonur.HPChargeLimit.Desktop.installer.yaml"
        )
        for expected in (
            f'InstallerSha256: "{desktop_hash}"',
            f'ProductCode: "{generate.DESKTOP_PRODUCT_CODE}"',
            f'DisplayName: "{generate.DESKTOP_DISPLAY_NAME}"',
            'InstallerType: "inno"',
            "https://github.com/kralonur/hp-charge-limit-rs/releases/download/"
            f"v{VERSION}/{DESKTOP_EXE}",
        ):
            self.assertIn(expected, desktop)

        version_manifest = self._read(CLI_DIR / "kralonur.HPChargeLimit.yaml")
        self.assertIn('ManifestType: "version"', version_manifest)
        self.assertIn('ManifestVersion: "1.6.0"', version_manifest)

        cli_locale = self._read(
            CLI_DIR / "kralonur.HPChargeLimit.locale.en-US.yaml"
        )
        desktop_locale = self._read(
            DESKTOP_DIR / "kralonur.HPChargeLimit.Desktop.locale.en-US.yaml"
        )
        self.assertIn('PackageName: "HP Charge Limit"', cli_locale)
        self.assertIn('PackageName: "HP Charge Limit Desktop"', desktop_locale)
        for expected in (
            'License: "MIT OR Apache-2.0"',
            "Windows support is experimental",
            "compatibility must be verified",
        ):
            self.assertIn(expected, cli_locale)
            self.assertIn(expected, desktop_locale)
        self.assertIn(
            "toggles between limiting and full-charge bypass", desktop_locale
        )
        self.assertIn(
            "does not switch the firmware between auto, inhibit and hold",
            desktop_locale,
        )
        self.assertNotIn("Copyright", cli_locale + desktop_locale)
        self.assertNotIn("Author:", cli_locale + desktop_locale)

    def test_prerelease_and_fork_urls(self):
        version = "0.4.1-beta.1"
        self.cli_zip.unlink()
        self.desktop_exe.rename(self.dist / generate.DESKTOP_EXE.format(version=version))
        with zipfile.ZipFile(self.dist / generate.CLI_ZIP.format(version=version), "w") as archive:
            archive.writestr(generate.CLI_EXE.format(version=version), b"cli")
        code, _, stderr = self._run(version=version, repository="example/fork")
        self.assertEqual(code, 0, stderr)
        files = list(self.out.rglob("*.yaml"))
        self.assertEqual(len(files), 6)
        for path in files:
            self.assertEqual(path.parent.name, version)
            if path.name.endswith(".installer.yaml"):
                self.assertIn(f"https://github.com/example/fork/releases/download/v{version}/", path.read_text())

    def test_invalid_input(self):
        for version in ("0.4", "v0.4.0", "0.4.0+meta", "../0.4.0", "0.4.0/1", "00.4.0", "0.4.0-" + "a" * 128):
            with self.subTest(version=version):
                self._reject(version=version, text="version")
        for repository in (
            "kralonur",
            "kralonur/repo/extra",
            "../repo",
            "kralonur/../repo",
            "kralonur/re po",
            "kralonur/.",
        ):
            with self.subTest(repository=repository):
                self._reject(repository=repository, text="repository")

    def test_missing_and_bad_artifacts(self):
        self.desktop_exe.unlink()
        self._reject(text=DESKTOP_EXE)
        self.cli_zip.unlink()
        stderr = self._reject()
        self.assertIn(CLI_ZIP, stderr)
        self.assertIn(DESKTOP_EXE, stderr)

        self._write_cli_zip()
        self.desktop_exe.write_bytes(b"desktop")
        self.cli_zip.write_bytes(b"not a zip")
        self._reject(text="ZIP")

        self._write_cli_zip(nested="wrong/hp-charge-limit-rs.exe")
        self._reject(text=CLI_NESTED)

        self._write_cli_zip(data=b"")
        self._reject(text="empty")

        self._write_cli_zip()
        self.desktop_exe.write_bytes(b"")
        self._reject(text="empty")

    def test_desktop_iss_matches_manifest(self):
        iss = (HERE.parent / "windows" / "desktop.iss").read_text(encoding="utf-8")

        def value(key):
            for line in iss.splitlines():
                if line.startswith(key + "="):
                    return line.split("=", 1)[1].strip()
            self.fail(f"{key} missing from desktop.iss")

        self.assertEqual(value("AppId").replace("{{", "{"), generate.DESKTOP_APP_ID)
        self.assertEqual(value("UninstallDisplayName"), generate.DESKTOP_DISPLAY_NAME)
        self.assertEqual(value("AppName"), generate.DESKTOP_DISPLAY_NAME)
        self.assertEqual(value("AppPublisher"), generate.PUBLISHER)
        self.assertEqual(value("PrivilegesRequired"), "lowest")

        code, _, stderr = self._run()
        self.assertEqual(code, 0, stderr)
        desktop = self._read(
            DESKTOP_DIR / "kralonur.HPChargeLimit.Desktop.installer.yaml"
        )
        self.assertIn(f'ProductCode: "{generate.DESKTOP_PRODUCT_CODE}"', desktop)
        self.assertIn('Scope: "user"', desktop)

    @unittest.skipUnless(yaml and jsonschema, "PyYAML/jsonschema not installed")
    def test_official_schema(self):
        schema_dir = os.environ.get("WINGET_SCHEMA_DIR")
        if not schema_dir:
            self.skipTest("set WINGET_SCHEMA_DIR to the official 1.6.0 schemas")
        schemas = {
            "version": "manifest.version.1.6.0.json",
            "installer": "manifest.installer.1.6.0.json",
            "locale": "manifest.locale.1.6.0.json",
        }
        code, _, stderr = self._run()
        self.assertEqual(code, 0, stderr)
        for path in self.out.rglob("*.yaml"):
            data = yaml.safe_load(path.read_text(encoding="utf-8"))
            schema = json.loads(
                (Path(schema_dir) / schemas[data["ManifestType"]]).read_text(
                    encoding="utf-8"
                )
            )
            jsonschema.validate(data, schema)


if __name__ == "__main__":
    unittest.main()
