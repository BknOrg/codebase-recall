$ErrorActionPreference = 'Stop'

$Repo = "BknOrg/codebase-recall"
$BinName = "code-rcl"
$PackageName = "codebase-recall"
$Target = "x86_64-pc-windows-msvc"
$InstallDir = "$HOME\.local\bin"

if (!(Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

$AssetUrl = "https://github.com/$Repo/releases/latest/download/$PackageName-$Target.zip"
$ZipPath = "$env:TEMP\$PackageName.zip"

Write-Host "Downloading $PackageName..."
Invoke-WebRequest -Uri $AssetUrl -OutFile $ZipPath

Expand-Archive -Path $ZipPath -DestinationPath $InstallDir -Force
Remove-Item $ZipPath

Write-Host "Berhasil! $BinName terpasang di $InstallDir\$BinName.exe"