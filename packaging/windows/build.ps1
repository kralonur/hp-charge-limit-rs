#Requires -Version 5.1
# Build an unsigned installer from already-built x64 Windows binaries.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BinaryDir,
    [string]$OutputDir = 'dist'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$binaries = (Resolve-Path -LiteralPath $BinaryDir).Path
foreach ($name in @('hp-charge-limit-rs.exe', 'hp-charge-limit-gui.exe', 'hp-charge-limit-tray.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $binaries $name) -PathType Leaf)) {
        throw "Missing $name in $binaries"
    }
}
$version = & python -c "import sys, tomllib; print(tomllib.load(open(sys.argv[1], 'rb'))['package']['version'])" (Join-Path $repo 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'Could not read the Cargo version with Python 3.11+.' }
$version = $version.Trim()
if ($version -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
    throw "Invalid package version: $version"
}
$fileVersion = "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"

$compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
if ($compiler) {
    $compiler = $compiler.Source
} else {
    $compiler = @(
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (-not $compiler) {
    throw 'Install Inno Setup 6.3 or newer, or put ISCC.exe on PATH.'
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$output = (Resolve-Path -LiteralPath $OutputDir).Path
& $compiler "/DAppVersion=$version" "/DFileVersion=$fileVersion" "/DBinaryDir=$binaries" "/DRepoDir=$repo" "/O$output" (Join-Path $PSScriptRoot 'desktop.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE." }

$name = "hp-charge-limit-desktop-v$version-x86_64-setup.exe"
$installer = Join-Path $output $name
if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) { throw "Missing installer: $installer" }
$hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
Set-Content -LiteralPath "$installer.sha256" -Encoding ASCII -NoNewline -Value "$hash  $name`n"
Write-Host "Built unsigned installer: $installer"
