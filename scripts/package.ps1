# Empaqueta EasyLiDAR para Windows: compila en release y crea dist\EasyLiDAR-<versión>-windows-x64.zip
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$env:Path += ";$env:USERPROFILE\.cargo\bin"
cargo build --release --locked
$v = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "EasyLiDAR-$v-windows-x64"
$d = Join-Path $root "dist\$name"
if (Test-Path $d) { Remove-Item $d -Recurse -Force }
New-Item -ItemType Directory -Path $d | Out-Null
Copy-Item target\release\easylidar.exe, target\release\easylidar-gui.exe $d
Copy-Item README.md, LICENSE, NOTICE, THIRD_PARTY_LICENSES.md $d
Copy-Item docs\MANUAL.md $d
Copy-Item licenses $d -Recurse
Compress-Archive -Path $d -DestinationPath "dist\$name.zip" -Force
Remove-Item $d -Recurse -Force
Write-Host "dist\$name.zip"
