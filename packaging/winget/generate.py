#!/usr/bin/env python3
"""Generate unsigned WinGet manifests for hp-charge-limit-rs.

Reads the actual release assets, hashes them, and writes six fixed YAML
files. Nothing is submitted. See README.md for publishing.
"""

import argparse
import hashlib
import json
import re
import sys
import zipfile
from pathlib import Path

DEFAULT_REPOSITORY = "kralonur/hp-charge-limit-rs"
MANIFEST_VERSION = "1.6.0"
DEFAULT_LOCALE = "en-US"
PUBLISHER = "kralonur"
LICENSE = "MIT OR Apache-2.0"

CLI_ID = "kralonur.HPChargeLimit"
DESKTOP_ID = "kralonur.HPChargeLimit.Desktop"
DESKTOP_APP_ID = "{9743971B-8E07-4828-A04B-C50B0B3D976C}"
DESKTOP_PRODUCT_CODE = DESKTOP_APP_ID + "_is1"
DESKTOP_DISPLAY_NAME = "HP Charge Limit Desktop"
CLI_ZIP = "hp-charge-limit-rs-v{version}-x86_64-pc-windows-msvc.zip"
CLI_EXE = "hp-charge-limit-rs-v{version}-x86_64-pc-windows-msvc/hp-charge-limit-rs.exe"
DESKTOP_EXE = "hp-charge-limit-desktop-v{version}-x86_64-setup.exe"

_VERSION_RE = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
_REPO_RE = re.compile(r"^([A-Za-z0-9][A-Za-z0-9-]{0,38})/([A-Za-z0-9._-]{1,100})$")


def q(value: str) -> str:
    return json.dumps(value, ensure_ascii=False)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest().upper()


def validate(version: str, repository: str) -> None:
    if len(version) > 128 or not _VERSION_RE.fullmatch(version):
        raise ValueError(
            f"invalid version {version!r}: expected MAJOR.MINOR.PATCH "
            "with an optional prerelease suffix"
        )
    match = _REPO_RE.fullmatch(repository)
    if not match or match.group(2) in {".", ".."} or ".." in match.group(2):
        raise ValueError(f"invalid repository {repository!r}: expected OWNER/REPO")


def check_cli_zip(path: Path, nested: str) -> None:
    try:
        with zipfile.ZipFile(path) as archive:
            info = archive.getinfo(nested)
            if info.is_dir() or info.file_size == 0:
                raise ValueError(f"{path}: nested installer is empty")
    except KeyError:
        raise ValueError(f"{path}: missing nested installer {nested!r}") from None
    except zipfile.BadZipFile as exc:
        raise ValueError(f"{path}: not a valid ZIP archive ({exc})") from exc


def version_manifest(package_id: str, version: str) -> str:
    return "\n".join([
        f"PackageIdentifier: {q(package_id)}",
        f"PackageVersion: {q(version)}",
        f"DefaultLocale: {q(DEFAULT_LOCALE)}",
        'ManifestType: "version"',
        f"ManifestVersion: {q(MANIFEST_VERSION)}",
    ]) + "\n"


def cli_installer(version: str, repository: str, archive: str, digest: str) -> str:
    nested = CLI_EXE.format(version=version)
    url = f"https://github.com/{repository}/releases/download/v{version}/{archive}"
    return "\n".join([
        f"PackageIdentifier: {q(CLI_ID)}",
        f"PackageVersion: {q(version)}",
        'InstallerType: "zip"',
        'NestedInstallerType: "portable"',
        "NestedInstallerFiles:",
        f"- RelativeFilePath: {q(nested)}",
        f"  PortableCommandAlias: {q('hp-charge-limit-rs')}",
        'Scope: "user"',
        'UpgradeBehavior: "install"',
        "Installers:",
        '- Architecture: "x64"',
        f"  InstallerUrl: {q(url)}",
        f"  InstallerSha256: {q(digest)}",
        'ManifestType: "installer"',
        f"ManifestVersion: {q(MANIFEST_VERSION)}",
    ]) + "\n"


def desktop_installer(version: str, repository: str, installer: str, digest: str) -> str:
    url = f"https://github.com/{repository}/releases/download/v{version}/{installer}"
    return "\n".join([
        f"PackageIdentifier: {q(DESKTOP_ID)}",
        f"PackageVersion: {q(version)}",
        'InstallerType: "inno"',
        'Scope: "user"',
        f"ProductCode: {q(DESKTOP_PRODUCT_CODE)}",
        "InstallModes:",
        '- "interactive"',
        '- "silent"',
        '- "silentWithProgress"',
        "InstallerSwitches:",
        f"  Silent: {q('/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-')}",
        f"  SilentWithProgress: {q('/SILENT /SUPPRESSMSGBOXES /NORESTART /SP-')}",
        'UpgradeBehavior: "install"',
        "AppsAndFeaturesEntries:",
        f'- DisplayName: {q(DESKTOP_DISPLAY_NAME)}',
        f"  Publisher: {q(PUBLISHER)}",
        f"  DisplayVersion: {q(version)}",
        f"  ProductCode: {q(DESKTOP_PRODUCT_CODE)}",
        '  InstallerType: "inno"',
        "Installers:",
        '- Architecture: "x64"',
        f"  InstallerUrl: {q(url)}",
        f"  InstallerSha256: {q(digest)}",
        'ManifestType: "installer"',
        f"ManifestVersion: {q(MANIFEST_VERSION)}",
    ]) + "\n"


def locale_manifest(
    package_id: str,
    version: str,
    repository: str,
    name: str,
    short: str,
    description: str,
    tags: list[str],
) -> str:
    lines = [
        f"PackageIdentifier: {q(package_id)}",
        f"PackageVersion: {q(version)}",
        f"PackageLocale: {q(DEFAULT_LOCALE)}",
        f"Publisher: {q(PUBLISHER)}",
        f"PackageName: {q(name)}",
        f"License: {q(LICENSE)}",
        f"LicenseUrl: {q(f'https://github.com/{repository}/blob/v{version}/README.md#license')}",
        f"ShortDescription: {q(short)}",
        f"Description: {q(description)}",
        "Tags:",
        *[f"- {q(tag)}" for tag in tags],
        'ManifestType: "locale"',
        f"ManifestVersion: {q(MANIFEST_VERSION)}",
    ]
    return "\n".join(lines) + "\n"


def outputs(
    version: str,
    repository: str,
    cli_archive: str,
    cli_digest: str,
    desktop_name: str,
    desktop_digest: str,
) -> dict[Path, str]:
    cli_dir = Path("manifests/k/kralonur/HPChargeLimit") / version
    desktop_dir = Path("manifests/k/kralonur/HPChargeLimit/Desktop") / version
    return {
        cli_dir / "kralonur.HPChargeLimit.yaml": version_manifest(CLI_ID, version),
        cli_dir / "kralonur.HPChargeLimit.installer.yaml": cli_installer(
            version, repository, cli_archive, cli_digest
        ),
        cli_dir / "kralonur.HPChargeLimit.locale.en-US.yaml": locale_manifest(
            CLI_ID,
            version,
            repository,
            "HP Charge Limit",
            "Command-line HP battery charge limiter for Windows (experimental).",
            "Command-line tool that sets the HP battery charge limit through the "
            "firmware's ACPI WMI interface. Windows support is experimental: "
            "compatibility must be verified on the specific HP model before use. "
            "Provides auto, inhibit and hold modes, diagnostics, and Windows "
            "service commands.",
            ["hp", "battery", "charge-limit", "power-management", "windows", "cli"],
        ),
        desktop_dir / "kralonur.HPChargeLimit.Desktop.yaml": version_manifest(
            DESKTOP_ID, version
        ),
        desktop_dir / "kralonur.HPChargeLimit.Desktop.installer.yaml": desktop_installer(
            version, repository, desktop_name, desktop_digest
        ),
        desktop_dir / "kralonur.HPChargeLimit.Desktop.locale.en-US.yaml": locale_manifest(
            DESKTOP_ID,
            version,
            repository,
            "HP Charge Limit Desktop",
            "Tray app and dashboard for the HP battery charge limit (experimental).",
            "Tray app and dashboard for the HP battery charge limit. It shows "
            "battery state and toggles between limiting and full-charge bypass; "
            "it does not switch the firmware between auto, inhibit and hold. "
            "Windows support is experimental: compatibility must be verified on "
            "the specific HP model. The background service is installed "
            "separately with the CLI.",
            ["hp", "battery", "charge-limit", "power-management", "windows", "gui", "tray"],
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Generate unsigned WinGet manifests.")
    parser.add_argument("--version", required=True)
    parser.add_argument("--artifacts", default="dist")
    parser.add_argument("--output", default="winget-manifests")
    parser.add_argument("--repository", default=DEFAULT_REPOSITORY)
    args = parser.parse_args(argv)

    try:
        version = args.version.strip()
        repository = args.repository.strip()
        validate(version, repository)

        artifacts = Path(args.artifacts)
        cli_zip = artifacts / CLI_ZIP.format(version=version)
        desktop_exe = artifacts / DESKTOP_EXE.format(version=version)
        missing = [str(path) for path in (cli_zip, desktop_exe) if not path.is_file()]
        if missing:
            raise ValueError("missing release artifact(s): " + ", ".join(missing))
        if cli_zip.stat().st_size == 0 or desktop_exe.stat().st_size == 0:
            raise ValueError("release artifacts must not be empty")
        check_cli_zip(cli_zip, CLI_EXE.format(version=version))

        files = outputs(
            version,
            repository,
            cli_zip.name,
            sha256(cli_zip),
            desktop_exe.name,
            sha256(desktop_exe),
        )
        output = Path(args.output)
        for relative, text in files.items():
            target = output / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8", newline="\n")
    except (ValueError, OSError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    for relative in files:
        print(output / relative)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
