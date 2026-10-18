#!/usr/bin/env bash
# Empaqueta EasyLiDAR en Linux o macOS: compila en release y crea dist/EasyLiDAR-<versión>-<sistema>-<arquitectura>.tar.gz
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
os=$(uname -s | tr 'A-Z' 'a-z'); [ "$os" = darwin ] && os=macos
name="EasyLiDAR-$v-$os-$(uname -m)"
d="dist/$name"
rm -rf "$d"; mkdir -p "$d"
cp target/release/easylidar target/release/easylidar-gui "$d"/
cp README.md LICENSE NOTICE THIRD_PARTY_LICENSES.md "$d"/
cp docs/MANUAL.md "$d"/
cp -r licenses "$d"/
chmod +x "$d"/easylidar "$d"/easylidar-gui
tar czf "dist/$name.tar.gz" -C dist "$name"
rm -rf "$d"
echo "dist/$name.tar.gz"
