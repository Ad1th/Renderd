# package.ps1 - Assembles renderd-viewer standalone executable package for Windows distribution.
Param(
    [string]$Profile = "release",
    [string]$Target = ""
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Resolve-Path "$ScriptDir\..\.."

if ($Target -ne "") {
    $BinaryPath = "$WorkspaceRoot\target\$Target\$Profile\renderd-viewer.exe"
} else {
    $BinaryPath = "$WorkspaceRoot\target\$Profile\renderd-viewer.exe"
}

if (-not (Test-Path $BinaryPath)) {
    if (Test-Path "$WorkspaceRoot\target\debug\renderd-viewer.exe") {
        $BinaryPath = "$WorkspaceRoot\target\debug\renderd-viewer.exe"
    }
}

$DistDir = "$WorkspaceRoot\target\dist"
$PackageZip = "$DistDir\renderd-viewer-windows.zip"

Write-Host "==> Packaging renderd-viewer executable"
Write-Host "    Binary source: $BinaryPath"
Write-Host "    Destination:   $PackageZip"

if (-not (Test-Path $BinaryPath)) {
    Write-Error "Error: Binary not found at $BinaryPath. Build renderd-viewer first."
    exit 1
}

New-Item -ItemType Directory -Force -Path $DistDir | Out-Null
Compress-Archive -Path $BinaryPath -DestinationPath $PackageZip -Force

Write-Host "==> renderd-viewer package assembled successfully at $PackageZip"
