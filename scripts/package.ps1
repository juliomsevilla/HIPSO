# Empaqueta EasyLiDAR para Windows: compila en release y crea dist\EasyLiDAR-windows-x64.zip
# (el mismo nombre que enlaza el README).
# Con -Publish sube el paquete a la release vX.Y.Z de GitHub (la crea si no existe; requiere `gh`).
param([switch]$Publish)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$env:Path += ";$env:USERPROFILE\.cargo\bin"
cargo build --release --locked
$v = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "EasyLiDAR-windows-x64"
New-Item -ItemType Directory -Path dist -Force | Out-Null
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
if ($Publish) {
  $tag = "v$v"
  gh release view $tag *> $null
  if ($LASTEXITCODE -ne 0) { gh release create $tag --target main --title $tag --generate-notes }
  gh release upload $tag "dist\$name.zip" --clobber
  Write-Host "Subido a la release $tag"
}
