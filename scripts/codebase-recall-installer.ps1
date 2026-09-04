$ErrorActionPreference = 'Stop'

$Repo = "BknOrg/codebase-recall"
$BinName = "codebase-recall"
$Target = "x86_64-pc-windows-msvc"
$InstallDir = "$HOME\.local\bin"

if (!(Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}

$AssetUrl = "https://github.com/$Repo/releases/latest/download/$BinName-$Target.zip"
$ZipPath = "$env:TEMP\$BinName.zip"

Write-Host "Downloading $BinName..."
Invoke-WebRequest -Uri $AssetUrl -OutFile $ZipPath

Expand-Archive -Path $ZipPath -DestinationPath $InstallDir -Force
Remove-Item $ZipPath

Write-Host "$BinName installed successfully to $InstallDir\$BinName.exe"