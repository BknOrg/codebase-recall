$ErrorActionPreference = 'Stop'

$Repo = "BknOrg/codebase-recall"
$BinName = "code-rcl"
$PackageName = "codebase-recall"
$Target = "x86_64-pc-windows-msvc"
$BaseDir = "$HOME\.code-rcl"
$InstallDir = "$BaseDir\bin"
$PluginDir = "$BaseDir\plugins"

if (!(Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}
if (!(Test-Path $PluginDir)) {
    New-Item -ItemType Directory -Force -Path $PluginDir | Out-Null
}

$AssetUrl = "https://github.com/$Repo/releases/latest/download/$PackageName-$Target.zip"
$ZipPath = "$env:TEMP\$PackageName.zip"

Write-Host "Downloading $PackageName..."
Invoke-WebRequest -Uri $AssetUrl -OutFile $ZipPath

Expand-Archive -Path $ZipPath -DestinationPath $InstallDir -Force
Remove-Item $ZipPath

Write-Host "Berhasil! $BinName terpasang di $InstallDir\$BinName.exe"

# --- Automatic add to PATH Windows ---
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ([string]::IsNullOrWhiteSpace($UserPath)) {
    [Environment]::SetEnvironmentVariable("Path", $InstallDir, "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "Berhasil menambahkan $InstallDir ke User PATH."
} elseif ($UserPath -notlike "*$InstallDir*") {
    $NewPath = if ($UserPath.TrimEnd().EndsWith(';')) { "$UserPath$InstallDir" } else { "$UserPath;$InstallDir" }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "Berhasil menambahkan $InstallDir ke User PATH."
} else {
    Write-Host "$InstallDir sudah ada di User PATH."
}

Write-Host "`nSelesai! Buka terminal baru dan jalankan: $BinName --help"