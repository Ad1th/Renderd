# package.ps1 - Zips the desktop app with the viewer for Windows.
#
# The app looks for renderd-viewer.exe beside its own executable, so both go
# into the same folder of the archive.
Param(
    [string]$Profile = "release",
    [string]$Target = ""
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Resolve-Path "$ScriptDir\..\.."

if ($Target -ne "") {
    $BinDir = "$WorkspaceRoot\target\$Target\$Profile"
} else {
    $BinDir = "$WorkspaceRoot\target\$Profile"
}

$Binaries = @("renderd.exe", "renderd-viewer.exe")
foreach ($Binary in $Binaries) {
    if (-not (Test-Path "$BinDir\$Binary")) {
        Write-Error "Error: $BinDir\$Binary not found. Build it first: cargo build --$Profile -p renderd-desktop -p renderd-viewer"
        exit 1
    }
}

$DistDir = "$WorkspaceRoot\target\dist"
$StageDir = "$DistDir\Renderd"
$PackageZip = "$DistDir\renderd-desktop-windows.zip"

Write-Host "==> Packaging the Renderd desktop app"
Write-Host "    Binaries:    $BinDir"
Write-Host "    Destination: $PackageZip"

if (Test-Path $StageDir) {
    Remove-Item -Recurse -Force $StageDir
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
foreach ($Binary in $Binaries) {
    Copy-Item "$BinDir\$Binary" "$StageDir\$Binary"
}

Compress-Archive -Path $StageDir -DestinationPath $PackageZip -Force

Write-Host "==> Package assembled at $PackageZip"
