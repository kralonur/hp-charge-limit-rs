# WinGet manifests

Unsigned manifests for:

- `kralonur.HPChargeLimit` — existing CLI ZIP, nested portable alias
  `hp-charge-limit-rs`, user scope.
- `kralonur.HPChargeLimit.Desktop` — unsigned Inno Setup installer, user scope.

Schema version is `1.6.0`. Nothing is submitted or listed automatically.

## Generate

```sh
python3 packaging/winget/generate.py \
  --version <version> \
  --artifacts dist \
  --output winget-manifests \
  --repository kralonur/hp-charge-limit-rs
```

`<version>` must match `Cargo.toml` and the release tag `v<version>`; use
`0.4.1` as the next-version example. Defaults are `dist`, `winget-manifests`,
and the canonical repository. Use `--repository OWNER/REPO` for a fork; the
package IDs and publisher stay canonical.

There is no `--tag`: URLs always use `v<version>`. Both assets must exist, and
the CLI ZIP must contain the executable at
`hp-charge-limit-rs-v<version>-x86_64-pc-windows-msvc/hp-charge-limit-rs.exe`.
SHA-256 is streamed from the actual files before any manifest is written.

## Output (six files)

```text
winget-manifests/manifests/k/kralonur/HPChargeLimit/<version>/
  kralonur.HPChargeLimit.yaml
  kralonur.HPChargeLimit.installer.yaml
  kralonur.HPChargeLimit.locale.en-US.yaml
winget-manifests/manifests/k/kralonur/HPChargeLimit/Desktop/<version>/
  kralonur.HPChargeLimit.Desktop.yaml
  kralonur.HPChargeLimit.Desktop.installer.yaml
  kralonur.HPChargeLimit.Desktop.locale.en-US.yaml
```

## Release and submit

1. Publish the GitHub release with both assets:
   - `hp-charge-limit-rs-v<version>-x86_64-pc-windows-msvc.zip`
   - `hp-charge-limit-desktop-v<version>-x86_64-setup.exe`
2. Download and extract `hp-charge-limit-winget-v<version>.zip` from the same
   release, or download both public assets into a clean `dist/` and run the
   generator. Verify the release checksums before submission.
3. On Windows, enable local manifests once (Administrator shell), then
   validate and test installation as your normal desktop user:
   ```powershell
   winget settings --enable LocalManifestFiles
   winget validate --manifest .\winget-manifests\manifests\k\kralonur\HPChargeLimit\<version>
   winget validate --manifest .\winget-manifests\manifests\k\kralonur\HPChargeLimit\Desktop\<version>
   winget install --manifest .\winget-manifests\manifests\k\kralonur\HPChargeLimit\<version>
   winget uninstall --id kralonur.HPChargeLimit
   winget install --manifest .\winget-manifests\manifests\k\kralonur\HPChargeLimit\Desktop\<version>
   winget uninstall --id kralonur.HPChargeLimit.Desktop
   ```
4. Submit each package separately; the community repository accepts one
   package version per pull request:
   ```powershell
   wingetcreate.exe submit --prtitle "New package: kralonur.HPChargeLimit <version>" --token $env:WINGET_CREATE_GITHUB_TOKEN .\winget-manifests\manifests\k\kralonur\HPChargeLimit\<version>
   wingetcreate.exe submit --prtitle "New package: kralonur.HPChargeLimit.Desktop <version>" --token $env:WINGET_CREATE_GITHUB_TOKEN .\winget-manifests\manifests\k\kralonur\HPChargeLimit\Desktop\<version>
   ```
   Never commit a token. Prefer `wingetcreate.exe token --store` and omit
   `--token`, or use `WINGET_CREATE_GITHUB_TOKEN`. A classic PAT with
   `public_repo` is required.
5. Wait for review. Do not call either package listed until the PR is merged.

## Desktop package

- AppId `{9743971B-8E07-4828-A04B-C50B0B3D976C}`, ProductCode
  `{9743971B-8E07-4828-A04B-C50B0B3D976C}_is1`, display name
  `HP Charge Limit Desktop`, publisher `kralonur`.
- Per-user (`PrivilegesRequired=lowest`). It never installs, starts, or
  upgrades the Windows service. A service installed separately, including a
  protected service copy, is left untouched. See
  [service update instructions](../windows/README.md#installation-ownership).
- The GUI toggles limiting vs full-charge bypass; it does not switch the
  firmware between auto, inhibit and hold.
- Inno `FileVersion` is numeric and derived by `packaging/windows/build.ps1`;
  `AppVersion` keeps the SemVer string. The generator does not handle this.

## Caveats

- Windows support is experimental. Compatibility must be verified on the
  specific HP model. The model tests in the root README are Linux-only.
- Neither asset is Authenticode-signed; the manifests have no
  `SignatureSha256`. WinGet accepts unsigned installers, but SmartScreen may
  warn. Do not add fake hashes.
- To update, keep the package IDs, bump the version, publish new assets, and
  regenerate. The desktop AppId/ProductCode stay stable.

## Tests

```sh
python3 -m unittest discover -s packaging/winget -v
WINGET_SCHEMA_DIR=/tmp/wgschema python3 -m unittest discover -s packaging/winget -v
```

The schema check needs PyYAML and jsonschema; it is skipped otherwise.
