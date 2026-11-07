# Install the OpenVML local TTS runtime for the current Windows user.
param(
    [string]$DataDir = "$env:USERPROFILE\.openvml\runtime",
    [switch]$NoAutostart
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
if (Test-Path (Join-Path $Root "ovml-runtime.exe")) {
    $SourceDir = $Root
} elseif (Test-Path (Join-Path $Root "ovml-runtime\ovml-runtime.exe")) {
    $SourceDir = Join-Path $Root "ovml-runtime"
} else {
    throw "ovml-runtime.exe not found next to install.ps1"
}

$InstallDir = Join-Path $env:LOCALAPPDATA "OVML\runtime"
if (Test-Path $InstallDir) {
    Remove-Item -Recurse -Force $InstallDir
}
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -Path (Join-Path $SourceDir "*") -Destination $InstallDir -Recurse -Force
if (Test-Path (Join-Path $Root "uninstall.ps1")) {
    Copy-Item -Path (Join-Path $Root "uninstall.ps1") -Destination $InstallDir -Force
}
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

$Exe = Join-Path $InstallDir "ovml-runtime.exe"
if (-not $NoAutostart) {
    $Startup = [Environment]::GetFolderPath("Startup")
    $ShortcutPath = Join-Path $Startup "OpenVML Runtime.lnk"
    $Shell = New-Object -ComObject WScript.Shell
    $Shortcut = $Shell.CreateShortcut($ShortcutPath)
    $Shortcut.TargetPath = $Exe
    $Shortcut.Arguments = "--data-dir `"$DataDir`""
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.Description = "Local text-to-speech runtime for OpenVML Studio"
    $Shortcut.Save()
    Write-Host "Autostart: $ShortcutPath"
}

# Add install dir to user PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
    Write-Host "Added $InstallDir to user PATH."
}

Write-Host "Installed: $Exe"
Write-Host "Models and cache: $DataDir"
Write-Host "OpenVML Studio will detect the runtime automatically."
Write-Host ""
Write-Host "Open a new terminal or restart your computer for PATH changes to take effect."
