# Remove the OpenVML local TTS runtime installed by install.ps1.
param(
    [string]$DataDir = "$env:USERPROFILE\.openvml\runtime",
    [switch]$PurgeData
)

$ErrorActionPreference = "Stop"
Get-Process ovml-runtime -ErrorAction SilentlyContinue | Stop-Process -Force
$Startup = [Environment]::GetFolderPath("Startup")
Remove-Item -Force (Join-Path $Startup "OpenVML Runtime.lnk") -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force (Join-Path $env:LOCALAPPDATA "OVML\runtime") -ErrorAction SilentlyContinue

# Remove install dir from user PATH
$InstallDir = Join-Path $env:LOCALAPPDATA "OVML\runtime"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -like "*$InstallDir*") {
    $newPath = (($userPath -split ';') | Where-Object { $_ -ne $InstallDir }) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Removed $InstallDir from user PATH."
}
if ($PurgeData) {
    Remove-Item -Recurse -Force $DataDir -ErrorAction SilentlyContinue
    Write-Host "Removed data: $DataDir"
}
Write-Host "Uninstalled OpenVML runtime."
Write-Host "Models and cache kept in: $DataDir (use -PurgeData to remove)."
