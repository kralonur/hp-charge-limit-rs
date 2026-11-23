#Requires -Version 5.1
# Per-user desktop files only: no service changes, elevation, or tray autostart.
[CmdletBinding()]
param([switch]$Uninstall)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
if (-not $env:LOCALAPPDATA -or -not $env:APPDATA) {
    throw 'Run as the desktop user; LOCALAPPDATA and APPDATA must be set.'
}
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\hp-charge-limit-desktop'
$marker = Join-Path $installDir '.hp-charge-limit-desktop'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\HP Charge Limit'
$binaries = @('hp-charge-limit-rs.exe', 'hp-charge-limit-gui.exe', 'hp-charge-limit-tray.exe')
$shortcuts = @{
    'HP Charge Limit' = 'hp-charge-limit-gui.exe'
    'HP Charge Limit Tray' = 'hp-charge-limit-tray.exe'
}

if (@(Get-Process -Name 'hp-charge-limit-gui', 'hp-charge-limit-tray' -ErrorAction SilentlyContinue).Count) {
    throw 'Close the dashboard and tray before installing or removing desktop files. Leave the service running.'
}
if (Test-Path -LiteralPath $installDir) {
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf) -or
        (Get-Content -LiteralPath $marker -TotalCount 1) -ne 'hp-charge-limit-desktop') {
        throw "Refusing to replace an unrecognized installation at $installDir"
    }
}

if ($Uninstall) {
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) {
        throw 'No owned desktop installation found.'
    }
    foreach ($name in $shortcuts.Keys) {
        Remove-Item -LiteralPath (Join-Path $startMenu "$name.lnk") -Force -ErrorAction SilentlyContinue
    }
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    $runValue = 'HpChargeLimitTray'
    $expected = '"' + (Join-Path $installDir 'hp-charge-limit-tray.exe') + '"'
    $existing = Get-ItemPropertyValue -LiteralPath $runKey -Name $runValue -ErrorAction SilentlyContinue
    if ($existing -eq $expected) {
        Remove-ItemProperty -LiteralPath $runKey -Name $runValue
    }
    foreach ($binary in $binaries) {
        Remove-Item -LiteralPath (Join-Path $installDir $binary) -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -LiteralPath $marker -Force
    foreach ($directory in @($installDir, $startMenu)) {
        if ((Test-Path -LiteralPath $directory) -and
            @(Get-ChildItem -LiteralPath $directory -Force).Count -eq 0) {
            Remove-Item -LiteralPath $directory -Force
        }
    }
}
else {
    foreach ($binary in $binaries) {
        if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $binary) -PathType Leaf)) {
            throw "Missing $binary beside install.ps1; extract the whole bundle."
        }
    }
    & (Join-Path $PSScriptRoot 'hp-charge-limit-rs.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'The bundled CLI does not run on this system.' }
    New-Item -ItemType Directory -Force -Path $installDir, $startMenu | Out-Null
    foreach ($binary in $binaries) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $binary) -Destination (Join-Path $installDir $binary) -Force
    }
    Set-Content -LiteralPath $marker -Value 'hp-charge-limit-desktop' -Encoding ASCII
    $shell = New-Object -ComObject WScript.Shell
    foreach ($name in $shortcuts.Keys) {
        $shortcut = $shell.CreateShortcut((Join-Path $startMenu "$name.lnk"))
        $shortcut.TargetPath = Join-Path $installDir $shortcuts[$name]
        $shortcut.WorkingDirectory = $installDir
        $shortcut.Save()
    }
    Write-Host "Installed in $installDir. Open HP Charge Limit from the Start Menu."
}
Write-Host 'The system CLI, service and its settings were not changed. Tray login startup is opt-in.'
