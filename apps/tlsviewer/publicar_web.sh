#!/usr/bin/env bash
# Prepara una carpeta para publicar una nube convertida con el visor de HIPSO en una web estática.
# Uso: publicar_web.sh <carpeta del octree (la de metadata.json)> <carpeta de destino>
set -euo pipefail
[ $# -eq 2 ] || { echo "uso: $0 <carpeta del octree> <carpeta de destino>" >&2; exit 1; }
src="$1"; dst="$2"
[ -f "$src/metadata.json" ] || { echo "$src no tiene metadata.json: no es una nube convertida" >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$dst/data"
cp -R "$here/web/." "$dst/"
cp "$src/metadata.json" "$src/hierarchy.bin" "$src/octree.bin" "$dst/data/"
echo "Listo en $dst: sírvela con un servidor estático que admita peticiones Range."
