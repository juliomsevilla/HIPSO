#!/usr/bin/env bash
# Crea EasyLiDAR.app dentro de una carpeta de paquete de macOS: abierta con doble clic no abre la ventana
# de Terminal. Lo usan scripts/package.sh (en local) y .github/workflows/release.yml (en Actions).
# Uso: scripts/macos-app.sh <carpeta del paquete> <binario de la interfaz>
set -euo pipefail
cd "$(dirname "$0")/.."
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app="$(cd "$1" && pwd)/EasyLiDAR.app"
bin="$2"
ico="$(cd "$1" && pwd)/.icon.iconset"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$ico"
cp "$bin" "$app/Contents/MacOS/easylidar-gui"
chmod +x "$app/Contents/MacOS/easylidar-gui"
for s in 16 32 64 128 256 512; do sips -z $s $s assets/icon-512.png --out "$ico/icon_${s}x${s}.png" >/dev/null; done
iconutil -c icns "$ico" -o "$app/Contents/Resources/EasyLiDAR.icns"
rm -rf "$ico"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>EasyLiDAR</string>
  <key>CFBundleDisplayName</key><string>EasyLiDAR</string>
  <key>CFBundleIdentifier</key><string>es.easylidar.app</string>
  <key>CFBundleExecutable</key><string>easylidar-gui</string>
  <key>CFBundleIconFile</key><string>EasyLiDAR</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$v</string>
  <key>CFBundleVersion</key><string>$v</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
