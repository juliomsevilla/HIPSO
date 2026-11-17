#!/usr/bin/env python3
"""Genera docs/CLI.md: la referencia de cada herramienta de `hipso` con todas sus opciones.

Se ejecuta con el programa ya compilado (`cargo build --release`) y se vuelve a lanzar al preparar
cada versión. Lee la definición de las órdenes con `hipso schema` (las mismas opciones que usan la
interfaz y el constructor de flujos) y la ayuda larga de cada una (`hipso <orden> --help`) para
los valores posibles de cada opción, de modo que la referencia nunca se desfasa del programa. Lo
único que escribe a mano este script es el reparto en áreas y el botón de la cinta donde vive cada
herramienta.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
HIPSO = RAIZ / "target" / "release" / ("hipso.exe" if sys.platform == "win32" else "hipso")

# (título del área, órdenes)
GRUPOS = [
    ("Revisar y preparar nubes", "info catalog crs import dedup merge-clouds tile clip filter thin edit apply-edits colorize intensity swath flightlines compare fit hag"),
    ("Clasificar y modelar superficies", "classify dtm dsm chm hillshade contour raster metrics"),
    ("Vegetación e inventario", "trees obia stem-volume model change corridor"),
    ("Ráster", "stack mosaic align reproject cog reclass clip-raster sieve proximity fill-nodata calc vectorize rasterize focal rstats tabulate zonal"),
    ("Terreno e hidrología", "terrain ponds flood ridges leastcost piles cutfill"),
    ("Vectorial", "is-valid make-valid buffer dissolve union intersect voronoi voronoi-density extract join-table merge split clip-vector near summary topology geojson"),
    ("Salida y comprobación de nubes", "export copc render"),
]

# dónde está cada herramienta en la interfaz y dónde se explica su método
DONDE = {
    "info": ("panel lateral → Información", "4.9"),
    "crs": ("LiDAR → Preparar → Asignar proyección", "7.1"),
    "import": ("Abrir (un PLY o un E57 se convierte al abrirlo)", "10"),
    "dedup": ("LiDAR → Preparar → Quitar duplicados", "6.4"),
    "merge-clouds": ("LiDAR → Calidad → Unir nubes", "6.5"),
    "clip": ("LiDAR → ALS → Extraer muestras", "7.3"),
    "filter": ("LiDAR → Preparar → Exportar puntos (con normalizar)", "7.1"),
    "thin": ("LiDAR → Preparar → Adelgazar nube", "7.1"),
    "edit": ("panel lateral → Edición (nubes de puntos)", "6"),
    "apply-edits": ("panel lateral → Edición → Guardar ediciones… y Cargar ediciones…", "6"),
    "colorize": ("LiDAR → Preparar → Colorear con imagen", "7.1"),
    "intensity": ("LiDAR → Preparar → Normalizar intensidad", "7.1"),
    "swath": ("LiDAR → Calidad → Entre pasadas", "7.1"),
    "flightlines": ("LiDAR → Calidad → Líneas de vuelo", "7.1"),
    "compare": ("LiDAR → Calidad → Comparar nubes", "7.1"),
    "fit": ("LiDAR → Calidad → Ajustar nubes", "7.1"),
    "hag": ("LiDAR → Preparar → Alturas sobre suelo", "6.3"),
    "classify": ("LiDAR → Preparar → Clasificar", "7.1"),
    "dtm": ("LiDAR → Superficies → MDT", "7.2"),
    "dsm": ("LiDAR → Superficies → MDS / CHM", "7.2"),
    "chm": ("LiDAR → Superficies → MDS / CHM", "7.2"),
    "hillshade": ("sin botón propio (la línea de comandos y el constructor de flujos)", "7.2"),
    "contour": ("Vista → Herramientas → Curvas de nivel (en pantalla)", "5"),
    "raster": ("LiDAR → Preparar → Imagen de intensidad (la rejilla de intensidad media)", "7.1"),
    "metrics": ("LiDAR → ALS → Métricas por celda", "7.3"),
    "trees": ("LiDAR → ALS → Segmentar individuos", "7.3"),
    "obia": ("LiDAR → ALS → Análisis por objetos", "7.3"),
    "stem-volume": ("LiDAR → TLS → Medición de troncos", "7.3"),
    "model": ("LiDAR → ALS → Modelos lineales", "7.3"),
    "change": ("LiDAR → Superficies → Cambio de la cubierta vegetal", "7.3"),
    "corridor": ("LiDAR → Infraestructuras → Corredores eléctricos", "7.3"),
    "stack": ("Raster → Imagen → Crear multibanda", "8"),
    "mosaic": ("Raster → Imagen → Mosaico", "8"),
    "align": ("Raster → Imagen → Igualar rásteres", "8"),
    "reproject": ("Raster → Geoprocesos → Reproyectar", "8"),
    "cog": ("sin botón propio (la línea de comandos y el constructor de flujos)", "10"),
    "reclass": ("Raster → Geoprocesos → Reclasificar", "8"),
    "clip-raster": ("Raster → Geoprocesos → Cortar por máscara", "8"),
    "sieve": ("Raster → Geoprocesos → Tamizar", "8"),
    "proximity": ("Raster → Geoprocesos → Proximidad", "8"),
    "fill-nodata": ("Raster → Geoprocesos → Rellenar huecos", "8"),
    "calc": ("Raster → Geoprocesos → Calculadora", "8"),
    "vectorize": ("Raster → Geoprocesos → Vectorizar", "8"),
    "rasterize": ("Raster → Geoprocesos → Rasterizar", "8"),
    "focal": ("Raster → Geoprocesos → Filtro focal", "8"),
    "rstats": ("Raster → Geoprocesos → Estadísticas ráster", "8"),
    "tabulate": ("Raster → Geoprocesos → Tabular áreas", "8"),
    "zonal": ("Vectorial → Geoprocesos → Estadística zonal", "9"),
    "terrain": ("Raster → Análisis del terreno (menús Relieve, Forma del terreno, Hidrología y Visibilidad y rutas)", "7.2"),
    "ponds": ("Raster → Análisis del terreno → Hidrología → Zonas encharcadas", "7.2"),
    "flood": ("Raster → Análisis del terreno → Hidrología → Inundación", "7.2"),
    "ridges": ("Raster → Análisis del terreno → Forma del terreno → Crestas y drenaje", "7.2"),
    "leastcost": ("Raster → Análisis del terreno → Visibilidad y rutas → Ruta de menor coste", "7.2"),
    "piles": ("Raster → Análisis del terreno → Volúmenes → Cubicación de pilas", "7.2"),
    "cutfill": ("Raster → Análisis del terreno → Volúmenes → Desmonte y terraplén", "7.2"),
    "is-valid": ("Vectorial → Geometría → Comprobar validez", "9"),
    "make-valid": ("Vectorial → Geometría → Reparar geometrías", "9"),
    "buffer": ("Vectorial → Geoprocesos → Búfer", "9"),
    "dissolve": ("Vectorial → Geoprocesos → Disolver", "9"),
    "union": ("Vectorial → Geoprocesos → Unión", "9"),
    "intersect": ("Vectorial → Geoprocesos → Intersección", "9"),
    "voronoi": ("Vectorial → Geoprocesos → Voronoi", "9"),
    "voronoi-density": ("LiDAR → Calidad → Densidad de Voronoi", "7.1"),
    "extract": ("Vectorial → Capas y tablas → Extraer valores", "9"),
    "join-table": ("Vectorial → Capas y tablas → Unir tabla", "9"),
    "merge": ("Vectorial → Capas y tablas → Combinar capas", "9"),
    "split": ("Vectorial → Capas y tablas → Dividir por campo", "9"),
    "clip-vector": ("Vectorial → Capas y tablas → Recortar capa", "9"),
    "near": ("Vectorial → Capas y tablas → Cercanía", "9"),
    "summary": ("Vectorial → Capas y tablas → Resumen estadístico", "9"),
    "topology": ("panel lateral → Edición (capas vectoriales) → Comprobar topología", "9"),
    "geojson": ("sin botón propio (la línea de comandos y el constructor de flujos)", "10"),
    "export": ("LiDAR → Preparar → Exportar puntos", "7.1"),
    "copc": ("se crea sola al abrir una nube grande (véase Ajustes)", "4.10"),
    "render": ("Archivo → Guardar vista PNG (en la interfaz, lo que se ve)", "4.2"),
    "tile": ("sin botón propio (la línea de comandos y el constructor de flujos)", "12"),
    "catalog": ("sin botón propio (la línea de comandos y el constructor de flujos)", "12"),
}


def ejecutar(*args):
    return subprocess.run([str(HIPSO), *args], capture_output=True, text=True, check=True).stdout


def valores_con_ayuda(nombre):
    """{opción larga: [(valor, explicación)]} leído de `hipso <orden> --help`."""
    texto = ejecutar(nombre, "--help")
    res, actual, en_valores = {}, None, False
    for linea in texto.splitlines():
        m = re.match(r"^\s+(?:-\w, )?--(\S+)", linea)
        if m:
            actual, en_valores = m.group(1), False
            continue
        if "Possible values:" in linea:
            en_valores = True
            continue
        if en_valores:
            v = re.match(r"^\s+- ([^:]+?):\s+(.*)$", linea)
            if v and actual:
                res.setdefault(actual, []).append((v.group(1), v.group(2).strip()))
            elif not linea.strip():
                en_valores = False
    return res


def descripcion_larga(nombre):
    """Las líneas de ayuda que preceden al «Usage:» (la descripción de la orden)."""
    texto = ejecutar(nombre, "--help")
    return texto.split("Usage:")[0].strip()


def celda(t):
    return (t or "").replace("|", "\\|").replace("\n", " ").strip()


def tipo(a):
    if a["flag"]:
        return "casilla"
    if a["values"]:
        return "una de las opciones"
    if a["path"]:
        return "archivo o carpeta"
    d = a["default"]
    if d is not None:
        try:
            float(d)
            return "número"
        except ValueError:
            return "texto"
    return "texto o número"


def nombre_opcion(a):
    if a["positional"]:
        return f"`{a['id'].upper()}`"
    partes = []
    if a["short"]:
        partes.append(f"`-{a['short']}`")
    partes.append(f"`--{a['long']}`")
    return ", ".join(partes)


def tabla(t, valores):
    filas = ["| Opción | Valor | Por defecto | Qué hace |", "|---|---|---|---|"]
    for a in t["args"]:
        desc = celda(a["help"])
        if desc and not desc.endswith((".", ")", ":")):
            desc += "."
        if a["required"] and not a["positional"]:
            desc += " **Obligatoria.**"
        if a["positional"]:
            desc += " **Obligatoria.**" if a["required"] else ""
        if a["multiple"]:
            desc += " Se puede repetir."
        vals = [] if a["flag"] else valores.get(a["long"] or "", [])
        if a["flag"]:
            pass
        elif vals:
            desc += " Valores: " + "; ".join(f"`{v}`: {celda(e)}" if e else f"`{v}`" for v, e in vals) + "."
        elif a["values"]:
            desc += " Valores: " + ", ".join(f"`{v}`" for v in a["values"]) + "."
        if a["flag"]:
            por_defecto = "desactivada"
        else:
            por_defecto = f"`{a['default']}`" if a["default"] not in (None, "") else "ninguno"
        filas.append(f"| {nombre_opcion(a)} | {tipo(a)} | {por_defecto} | {desc} |")
    return filas


def main():
    esquema = {t["name"]: t for t in json.loads(ejecutar("schema"))["tools"]}
    usadas = set()
    out = [
        "# Referencia de herramientas",
        "",
        "Cada herramienta del programa es una orden de la línea de comandos, y la interfaz gráfica y el constructor de flujos ejecutan estas mismas órdenes. Por eso esta página sirve para las tres cosas: para cada herramienta dice qué hace, dónde está en la cinta y qué significa cada una de sus opciones, con su valor por defecto y los valores que admite. Las ventanas de la interfaz muestran estas mismas opciones, con la misma ayuda, y el constructor de flujos las ofrece como puertos y parámetros de cada nodo.",
        "",
        "La página se genera con `scripts/gen_cli_docs.py` a partir de la definición de las órdenes del propio programa, de modo que lista exactamente lo que admite la versión publicada. Todas las órdenes aceptan además `--help`, que imprime lo mismo en la terminal. Cuando una opción no lleva valor por defecto, la herramienta decide por sí misma o no actúa hasta que se le dé un valor.",
        "",
    ]
    for titulo, nombres in GRUPOS:
        out += [f"## {titulo}", ""]
        for n in nombres.split():
            if n not in esquema:
                continue
            usadas.add(n)
            t = esquema[n]
            donde, apartado = DONDE.get(n, ("sin botón propio", ""))
            out += [f"### hipso {n}", "", descripcion_larga(n), ""]
            out += [f"**En la interfaz:** {donde}." + (f" El método se explica en el apartado {apartado}." if apartado else ""), ""]
            out += tabla(t, valores_con_ayuda(n)) + [""]
    resto = [n for n in esquema if n not in usadas]
    if resto:
        out += ["## Otras", ""]
        for n in resto:
            t = esquema[n]
            out += [f"### hipso {n}", "", descripcion_larga(n), ""] + tabla(t, valores_con_ayuda(n)) + [""]
    out += [
        "## Flujos de trabajo", "",
        "Tres órdenes más no procesan datos, sino flujos del constructor (capítulo 13). `hipso schema` imprime en JSON la definición de todas las órdenes y opciones de esta página, que es lo que lee el constructor para ofrecer cada herramienta. `hipso run flujo.model.json` ejecuta un flujo guardado sin interfaz (con `--input nombre=ruta` para cambiar una entrada y `--results-dir` para la carpeta de resultados), y `hipso model-validate flujo.model.json` lista lo que impide ejecutarlo.", "",
    ]
    (RAIZ / "docs" / "CLI.md").write_text("\n".join(out), encoding="utf-8")
    print(f"{len(esquema)} órdenes, {len(resto)} sin grupo: {resto}")


if __name__ == "__main__":
    main()
