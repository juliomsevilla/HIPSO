#!/usr/bin/env bash
# Empaqueta EasyLiDAR en macOS o Linux: compila en release y crea dist/EasyLiDAR-<sistema>-<arquitectura>.tar.gz
# (los mismos nombres que enlaza el README). En un Mac con Apple Silicon crea también el paquete Intel
# (necesita `rustup target add x86_64-apple-darwin`).
# Con --publish sube los paquetes a la release vX.Y.Z de GitHub (la crea si no existe; requiere `gh`).
set -euo pipefail
cd "$(dirname "$0")/.."
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
os=$(uname -s | tr 'A-Z' 'a-z'); [ "$os" = darwin ] && os=macos
arch=$(uname -m); [ "$arch" = x86_64 ] && arch=x64
mkdir -p dist
packages=()

# En macOS la app va en un .app: abierta con doble clic no abre la ventana de Terminal.
app_bundle() { # <carpeta del paquete> <binario de la interfaz>
  local app="$1/EasyLiDAR.app" ico="$1/.icon.iconset"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$ico"
  cp "$2" "$app/Contents/MacOS/easylidar-gui"
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
}

pack() { # <nombre> <carpeta de salida de cargo>
  local name="EasyLiDAR-$1" d="dist/EasyLiDAR-$1"
  rm -rf "$d"; mkdir -p "$d"
  cp "$2/easylidar" "$2/easylidar-gui" "$d"/
  cp README.md LICENSE NOTICE THIRD_PARTY_LICENSES.md docs/MANUAL.md "$d"/
  cp -r licenses "$d"/
  chmod +x "$d"/easylidar "$d"/easylidar-gui
  if [ "$os" = macos ]; then app_bundle "$d" "$2/easylidar-gui"; fi
  tar czf "dist/$name.tar.gz" -C dist "$name"
  rm -rf "$d"
  echo "dist/$name.tar.gz"
  packages+=("dist/$name.tar.gz")
}

cargo build --release --locked
pack "$os-$arch" target/release
if [ "$os" = macos ] && [ "$arch" = arm64 ]; then
  cargo build --release --locked --target x86_64-apple-darwin
  pack macos-x64 target/x86_64-apple-darwin/release
fi

if [ "${1:-}" = "--publish" ]; then
  tag="v$v"
  gh release view "$tag" >/dev/null 2>&1 || gh release create "$tag" --target main --title "$tag" --generate-notes
  gh release upload "$tag" "${packages[@]}" --clobber
  echo "Subido a la release $tag"
fi
