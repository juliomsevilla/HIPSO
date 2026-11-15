#!/usr/bin/env python3
"""Genera docs/CLI.md con la ayuda de cada orden de `hipso`, agrupada por áreas de trabajo.

Se ejecuta con el programa ya compilado (`cargo build --release`) y se vuelve a lanzar al preparar
cada versión, de modo que la referencia de la página web nunca se desfasa de la línea de comandos.
"""
import re
import subprocess
import sys
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
HIPSO = RAIZ / "target" / "release" / ("hipso.exe" if sys.platform == "win32" else "hipso")

GRUPOS = [
    ("Revisar y preparar nubes", "info catalog crs import dedup tile clip filter thin edit colorize intensity swath flightlines compare fit hag"),
    ("Clasificar y modelar superficies", "classify dtm dsm chm hillshade contour raster metrics"),
    ("Vegetación e inventario", "trees obia stem-volume model change corridor"),
    ("Ráster", "stack mosaic align reproject cog reclass clip-raster sieve proximity fill-nodata calc vectorize rasterize focal rstats tabulate zonal"),
    ("Terreno e hidrología", "terrain ponds flood ridges leastcost piles cutfill"),
    ("Vectorial", "is-valid make-valid buffer dissolve union intersect voronoi voronoi-density extract join-table merge split clip-vector near summary topology geojson"),
    ("Salida y comprobación de nubes", "export copc render"),
    ("Flujos de trabajo", "schema run model-validate"),
]


def ejecutar(*args):
    return subprocess.run([str(HIPSO), *args], capture_output=True, text=True, check=True).stdout


def ordenes():
    salida = ejecutar("--help")
    bloque = salida.split("Commands:")[1].split("Options:")[0]
    res = {}
    for linea in bloque.splitlines():
        m = re.match(r"^  (\S+)\s{2,}(.*)$", linea)
        if m and m.group(1) != "help":
            res[m.group(1)] = m.group(2).strip()
    return res


def ayuda(nombre):
    texto = ejecutar(nombre, "--help")
    # la ayuda de `clap` trae sus propias líneas en inglés que no aportan nada
    texto = "\n".join(l for l in texto.splitlines() if "Print help" not in l)
    texto = texto.replace("Usage:", "Uso:").replace("Arguments:", "Argumentos:").replace("Options:", "Opciones:")
    return texto.strip()


def main():
    todas = ordenes()
    usadas = set()
    out = ["# Órdenes de `hipso`", "",
           "Cada herramienta del programa es una orden de la línea de comandos, y la interfaz gráfica y el constructor de flujos ejecutan estas mismas órdenes. Esta página se genera con `scripts/gen_cli_docs.py` a partir de la ayuda del propio programa, de modo que lista exactamente lo que admite la versión publicada.", ""]
    for titulo, nombres in GRUPOS:
        out += [f"## {titulo}", ""]
        for n in nombres.split():
            if n not in todas:
                continue
            usadas.add(n)
            out += [f"### hipso {n}", "", todas[n], "", "```", ayuda(n), "```", ""]
    resto = [n for n in todas if n not in usadas]
    if resto:
        out += ["## Otras", ""]
        for n in resto:
            out += [f"### hipso {n}", "", todas[n], "", "```", ayuda(n), "```", ""]
    (RAIZ / "docs" / "CLI.md").write_text("\n".join(out), encoding="utf-8")
    print(f"{len(todas)} órdenes, {len(resto)} sin grupo: {resto}")


if __name__ == "__main__":
    main()
