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
if ($PurgeData) {
    Remove-Item -Recurse -Force $DataDir -ErrorAction SilentlyContinue
    Write-Host "Removed data: $DataDir"
}
Write-Host "Uninstalled OpenVML runtime."
Write-Host "Models and cache kept in: $DataDir (use -PurgeData to remove)."
