#Requires -Version 5.1
# CI-only: installs/removes a test service and the per-user desktop package.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$BinaryDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run this destructive smoke test only on an isolated GitHub Actions Windows runner.' }
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$cli = Join-Path (Resolve-Path -LiteralPath $BinaryDir).Path 'hp-charge-limit-rs.exe'
$app = Join-Path $env:LOCALAPPDATA 'Programs\hp-charge-limit-desktop'
$shortcuts = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\HP Charge Limit'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{9743971B-8E07-4828-A04B-C50B0B3D976C}_is1'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'HpChargeLimitTray'
$protectedExe = Join-Path $env:ProgramFiles 'hp-charge-limit-rs\hp-charge-limit-rs.exe'
$uninstaller = Join-Path $app 'unins000.exe'

function Get-TrayStartup {
    if (Test-Path -LiteralPath $runKey) {
        return (Get-Item -LiteralPath $runKey).GetValue($runName)
    }
}

if ((Test-Path -LiteralPath $app) -or (Test-Path -LiteralPath $shortcuts) -or
    (Test-Path -LiteralPath $uninstallKey) -or (Test-Path -LiteralPath $protectedExe) -or
    (Get-Service -Name 'hp-charge-limit-rs' -ErrorAction SilentlyContinue) -or
    ($null -ne (Get-TrayStartup))) {
    throw 'Refusing to test over an existing installation, service or tray startup entry.'
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Run-Setup([string]$Executable, [string]$Label, [switch]$ShouldFail) {
    $log = Join-Path $env:RUNNER_TEMP "hp-charge-$Label.log"
    $process = Start-Process -FilePath $Executable -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$log`"") -Wait -PassThru
    if ($ShouldFail) {
        Assert-True ($process.ExitCode -ne 0) "$Label unexpectedly succeeded."
        return
    }
    if ($process.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log | Write-Host }
        throw "$Label failed with exit code $($process.ExitCode)."
    }
}

function Assert-NoLaunch {
    Assert-True (-not (Get-Process -Name 'hp-charge-limit-gui', 'hp-charge-limit-tray' -ErrorAction SilentlyContinue)) 'Installer launched the GUI or tray.'
}

function Assert-Installed {
    foreach ($name in @('hp-charge-limit-rs.exe', 'hp-charge-limit-gui.exe', 'hp-charge-limit-tray.exe')) {
        $installed = Join-Path $app $name
        Assert-True (Test-Path -LiteralPath $installed) "Missing $installed"
        Assert-True ((Get-FileHash -LiteralPath $installed).Hash -eq (Get-FileHash -LiteralPath (Join-Path $BinaryDir $name)).Hash) "Wrong installed bytes: $name"
    }
    foreach ($name in @('HP Charge Limit.lnk', 'HP Charge Limit Tray.lnk')) {
        Assert-True (Test-Path -LiteralPath (Join-Path $shortcuts $name)) "Missing shortcut: $name"
    }
    Assert-True (Test-Path -LiteralPath $uninstallKey) 'Missing per-user uninstall registration.'
    $registration = Get-ItemProperty -LiteralPath $uninstallKey
    Assert-True ($registration.DisplayName -eq 'HP Charge Limit Desktop') 'Unexpected WinGet detection name.'
    Assert-True ($registration.Publisher -eq 'kralonur') 'Unexpected WinGet detection publisher.'
    Assert-NoLaunch
}

function Assert-Removed {
    Assert-True (-not (Test-Path -LiteralPath $uninstallKey)) 'Uninstall registration remains.'
    foreach ($name in @('hp-charge-limit-rs.exe', 'hp-charge-limit-gui.exe', 'hp-charge-limit-tray.exe', 'unins000.exe')) {
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $app $name))) "Uninstall left $name"
    }
    foreach ($name in @('HP Charge Limit.lnk', 'HP Charge Limit Tray.lnk')) {
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $shortcuts $name))) "Uninstall left shortcut $name"
    }
    Assert-True (-not (Test-Path -LiteralPath $app)) 'Uninstall left the app directory.'
    Assert-True (-not (Test-Path -LiteralPath $shortcuts)) 'Uninstall left the Start Menu group.'
    Assert-NoLaunch
}

function Service-Snapshot {
    $service = Get-CimInstance Win32_Service -Filter "Name='hp-charge-limit-rs'"
    Assert-True ($null -ne $service) 'Test service disappeared.'
    return "$($service.PathName)|$($service.StartMode)|$($service.State)|$($service.ProcessId)|$((Get-FileHash -LiteralPath $protectedExe).Hash)"
}

$serviceCreated = $false
try {
    # Refuse an unrecognized directory without changing its contents.
    New-Item -Path $app -ItemType Directory -Force | Out-Null
    $sentinel = Join-Path $app 'user-file.txt'
    Set-Content -LiteralPath $sentinel -Value 'keep'
    Run-Setup $installerPath 'unrecognized-directory' -ShouldFail
    Assert-True ((Get-Content -LiteralPath $sentinel) -eq 'keep') 'Installer modified an unrecognized directory.'
    Assert-True (-not (Test-Path -LiteralPath $uninstallKey)) 'Refused install created uninstall registration.'
    Remove-Item -LiteralPath $app -Recurse -Force

    # A silent fresh install must not enable a service, start applications or add autorun.
    Run-Setup $installerPath 'fresh-install'
    Assert-Installed
    $statusJson = & (Join-Path $app 'hp-charge-limit-rs.exe') status --json --brief
    Assert-True ($LASTEXITCODE -eq 0) 'Bundled CLI could not query status after install.'
    $status = $statusJson | ConvertFrom-Json
    Assert-True ($status.api_version -eq 1 -and $status.service -eq 'missing') 'Bundled CLI did not report a fresh install.'
    Assert-True (-not (Get-Service -Name 'hp-charge-limit-rs' -ErrorAction SilentlyContinue)) 'Installer created a service.'
    Assert-True (-not (Test-Path -LiteralPath $protectedExe)) 'Installer created a protected service binary.'
    Assert-True ($null -eq (Get-TrayStartup)) 'Installer enabled tray startup.'

    # Deliberately create a service using custom options. No HP firmware needed.
    & $cli service install --stop 80 --start 75 --off-mode keep
    if ($LASTEXITCODE -ne 0) { throw 'Could not create the test service.' }
    $serviceCreated = $true
    (Get-Service -Name 'hp-charge-limit-rs').WaitForStatus([System.ServiceProcess.ServiceControllerStatus]::Running, [TimeSpan]::FromSeconds(20))
    $before = Service-Snapshot

    # Reinstall adopts the legacy script's files and preserves opt-in startup.
    $marker = Join-Path $app '.hp-charge-limit-desktop'
    Set-Content -LiteralPath $marker -Value 'hp-charge-limit-desktop' -Encoding ASCII
    $ownedRun = '"' + (Join-Path $app 'hp-charge-limit-tray.exe') + '"'
    New-Item -Path $runKey -Force | Out-Null
    New-ItemProperty -LiteralPath $runKey -Name $runName -Value $ownedRun -PropertyType String -Force | Out-Null
    Run-Setup $installerPath 'reinstall'
    Assert-Installed
    Assert-True (-not (Test-Path -LiteralPath $marker)) 'Legacy script still owns the installation.'
    Assert-True ((Get-TrayStartup) -eq $ownedRun) 'Reinstall changed tray startup.'
    Assert-True ((Service-Snapshot) -eq $before) 'Reinstall changed or restarted the service.'

    Run-Setup $uninstaller 'uninstall'
    Assert-Removed
    Assert-True ($null -eq (Get-TrayStartup)) 'Owned startup entry remains.'
    Assert-True ((Service-Snapshot) -eq $before) 'Uninstall changed or restarted the service.'

    # An autorun command belonging to a different installation must survive.
    $foreignRun = '"C:\another-install\hp-charge-limit-tray.exe"'
    New-ItemProperty -LiteralPath $runKey -Name $runName -Value $foreignRun -PropertyType String -Force | Out-Null
    New-Item -Path $app -ItemType Directory -Force | Out-Null
    Set-Content -LiteralPath $marker -Value 'hp-charge-limit-desktop' -Encoding ASCII
    Run-Setup $installerPath 'legacy-adoption-with-existing-service'
    Assert-Installed
    Assert-True (-not (Test-Path -LiteralPath $marker)) 'Legacy ownership marker remains after adoption.'
    Run-Setup $uninstaller 'uninstall-with-foreign-startup'
    Assert-Removed
    Assert-True ((Get-TrayStartup) -eq $foreignRun) 'Uninstall deleted a foreign startup entry.'
    Assert-True ((Service-Snapshot) -eq $before) 'Installation lifecycle changed the existing service.'
    Write-Host 'Unsigned installer smoke tests passed; the service and its custom arguments were preserved.'
} finally {
    if ($serviceCreated) { & $cli service uninstall }
    if (Test-Path -LiteralPath $uninstaller) { Run-Setup $uninstaller 'cleanup' }
    if ($null -ne (Get-TrayStartup)) { Remove-ItemProperty -LiteralPath $runKey -Name $runName }
}
