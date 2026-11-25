#!/usr/bin/env bash
# Prepara la carpeta que se entrega a un cliente: el visor de HIPSO y sus nubes, listo para abrir
# con doble clic y sin instalar nada (ni servidor ni navegador).
#
# Uso: scripts/visor_cliente.sh <carpeta de destino> "<título>" <nube>...
#   <nube> es un LAS/LAZ (se convierte con PotreeConverter: variable POTREE_CONVERTER o el del PATH)
#   o una carpeta ya convertida (la que tiene metadata.json, p. ej. la de la caché de HIPSO).
# Resultado, en macOS y Linux (en Windows se copia hipso-tls.exe con el mismo reparto):
#   destino/HIPSO Visor.app (o «HIPSO Visor» en Linux), destino/nubes/<nube>/..., destino/titulo.txt
# Variable HIPSO_TLS: el binario del visor (por defecto, target/release/hipso-tls).
set -euo pipefail
[ $# -ge 3 ] || { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 1; }
cd "$(dirname "$0")/.."
dst="$1"; titulo="$2"; shift 2
bin="${HIPSO_TLS:-target/release/hipso-tls}"
[ -x "$bin" ] || { echo "falta $bin: compila con «cargo build --release -p hipso-tls»" >&2; exit 1; }
conv="${POTREE_CONVERTER:-$(command -v PotreeConverter || true)}"
mkdir -p "$dst/nubes"
dst="$(cd "$dst" && pwd)"
printf '%s\n' "$titulo" > "$dst/titulo.txt"

for n in "$@"; do
  if [ -d "$n" ]; then
    [ -f "$n/metadata.json" ] || { echo "$n no tiene metadata.json" >&2; exit 1; }
    nombre="$(basename "$n")"
    mkdir -p "$dst/nubes/$nombre"
    cp "$n/metadata.json" "$n/hierarchy.bin" "$n/octree.bin" "$dst/nubes/$nombre/"
  else
    [ -n "$conv" ] || { echo "para convertir $n hace falta PotreeConverter (POTREE_CONVERTER)" >&2; exit 1; }
    nombre="$(basename "${n%.*}")"
    tmp="$(mktemp -d)"
    (cd "$tmp" && "$conv" "$(cd "$(dirname "$n")" && pwd)/$(basename "$n")" -o "$dst/nubes/$nombre" --encoding BROTLI --chunkdir "$tmp/trozos")
    rm -rf "$tmp" "$dst/nubes/$nombre/log.txt"
  fi
  echo "nube: $nombre"
done

if [ "$(uname)" = Darwin ]; then
  v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
  app="$dst/HIPSO Visor.app"
  rm -rf "$app"; ico="$dst/.icon.iconset"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$ico"
  cp "$bin" "$app/Contents/MacOS/hipso-tls"
  for s in 16 32 64 128 256 512; do sips -z $s $s assets/icon-512.png --out "$ico/icon_${s}x${s}.png" >/dev/null; done
  iconutil -c icns "$ico" -o "$app/Contents/Resources/HIPSO.icns"; rm -rf "$ico"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>HIPSO Visor</string>
  <key>CFBundleDisplayName</key><string>HIPSO Visor</string>
  <key>CFBundleIdentifier</key><string>es.hipso.visor</string>
  <key>CFBundleExecutable</key><string>hipso-tls</string>
  <key>CFBundleIconFile</key><string>HIPSO</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$v</string>
  <key>CFBundleVersion</key><string>$v</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
else
  cp "$bin" "$dst/HIPSO Visor"; chmod +x "$dst/HIPSO Visor"
fi
du -sh "$dst"
