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
    ("LiDAR · Preparar y clasificar", "classify crs export filter thin hag dedup colorize intensity raster propagate-class edit apply-edits"),
    ("LiDAR · Calidad", "compare fit swath flightlines voronoi-density merge-clouds dtm-validate"),
    ("LiDAR · Superficies", "dtm dsm chm change hillshade contour"),
    ("LiDAR · ALS, TLS e infraestructuras", "clip metrics model trees trees3d tree-metrics obia canopy-gaps voxel-metrics stem-volume corridor"),
    ("Ráster · Imagen y geoprocesos", "stack mosaic align reproject calc clip-raster reclass sieve proximity fill-nodata vectorize rasterize focal tabulate rstats cog"),
    ("Ráster · Análisis del terreno", "terrain ridges ponds flood leastcost cutfill piles"),
    ("Vectorial · Geometría y tabla", "is-valid make-valid topology fix-topology simplify parallel clip-vector extract join-table split fields"),
    ("Vectorial · Geoprocesos", "zonal buffer dissolve union intersect voronoi merge near summary geojson"),
    ("Datos, formatos y entregas", "info import catalog tile copc render"),
]

# dónde está cada herramienta en la interfaz y dónde se explica su método
DONDE = {
    "info": ("panel lateral → Información", "4.9"),
    "crs": ("LiDAR → Preparar → Asignar proyección", "7.1"),
    "import": ("Abrir (un PLY o un E57 se convierte al abrirlo)", "10"),
    "dedup": ("LiDAR → Preparar → Quitar duplicados", "7.1"),
    "merge-clouds": ("LiDAR → Calidad → Unir nubes", "7.3"),
    "clip": ("LiDAR → ALS → Extraer muestras", "7.5"),
    "filter": ("LiDAR → Preparar → Exportar puntos (con normalizar)", "7.1"),
    "thin": ("LiDAR → Preparar → Adelgazar nube", "7.1"),
    "edit": ("panel lateral → Edición (nubes de puntos) y LiDAR → Preparar → Editar con capas", "6"),
    "apply-edits": ("panel lateral → Edición → Guardar ediciones… y Cargar ediciones…", "6"),
    "propagate-class": ("LiDAR → Preparar → Propagar clase. Flujo típico: `thin`, `classify` sobre lo adelgazado y `propagate-class` sobre la nube completa", None),
    "colorize": ("LiDAR → Preparar → Colorear con imagen", "7.1"),
    "intensity": ("LiDAR → Preparar → Normalizar intensidad", "7.1"),
    "swath": ("LiDAR → Calidad → Entre pasadas", "7.3"),
    "flightlines": ("LiDAR → Calidad → Líneas de vuelo", "7.3"),
    "compare": ("LiDAR → Calidad → Comparar nubes", "7.3"),
    "fit": ("LiDAR → Calidad → Ajustar nubes", "7.3"),
    "hag": ("LiDAR → Preparar → Alturas sobre suelo", "7.1"),
    "classify": ("LiDAR → Preparar → Clasificar", "7.2"),
    "dtm": ("LiDAR → Superficies → MDT", "7.4"),
    "dsm": ("LiDAR → Superficies → MDS / CHM", "7.4"),
    "chm": ("LiDAR → Superficies → MDS / CHM", "7.4"),
    "hillshade": ("sin botón propio (la línea de comandos y el constructor de flujos)", "7.4"),
    "contour": ("Vista → Herramientas → Curvas de nivel (en pantalla)", "5"),
    "raster": ("LiDAR → Preparar → Imagen de intensidad (la rejilla de intensidad media)", "7.1"),
    "metrics": ("LiDAR → ALS → Métricas por celda", "7.5"),
    "trees": ("LiDAR → ALS → Segmentar con CHM", "7.5"),
    "obia": ("LiDAR → ALS → Análisis por objetos", "7.5"),
    "stem-volume": ("LiDAR → TLS → Medición de troncos", "7.6"),
    "model": ("LiDAR → ALS → Modelos lineales", "7.5"),
    "change": ("LiDAR → Superficies → Cambio de la cubierta vegetal", "7.4"),
    "corridor": ("LiDAR → Infraestructuras → Corredores eléctricos", "7.7"),
    "stack": ("Raster → Imagen → Crear multibanda", "8.1"),
    "mosaic": ("Raster → Imagen → Mosaico", "8.1"),
    "align": ("Raster → Imagen → Igualar rásteres", "8.1"),
    "reproject": ("Raster → Geoprocesos → Reproyectar (también capas vectoriales)", "8.2"),
    "cog": ("sin botón propio (la línea de comandos y el constructor de flujos)", "10"),
    "reclass": ("Raster → Geoprocesos → Reclasificar", "8.2"),
    "clip-raster": ("Raster → Geoprocesos → Cortar por máscara", "8.2"),
    "sieve": ("Raster → Geoprocesos → Tamizar", "8.2"),
    "proximity": ("Raster → Geoprocesos → Proximidad", "8.2"),
    "fill-nodata": ("Raster → Geoprocesos → Rellenar huecos", "8.2"),
    "calc": ("Raster → Geoprocesos → Calculadora", "8.2"),
    "vectorize": ("Raster → Geoprocesos → Vectorizar", "8.2"),
    "rasterize": ("Raster → Geoprocesos → Rasterizar", "8.2"),
    "focal": ("Raster → Geoprocesos → Filtro focal", "8.2"),
    "rstats": ("Archivo → Análisis → EDA → Estadísticas (con un ráster como fuente)", "5"),
    "tabulate": ("Raster → Geoprocesos → Tabular áreas", "8.2"),
    "zonal": ("Vectorial → Geoprocesos → Estadística zonal", "9.6"),
    "terrain": ("Raster → Análisis del terreno (menús Relieve, Forma del terreno, Hidrología y Visibilidad y rutas)", "8.3"),
    "ponds": ("Raster → Análisis del terreno → Hidrología → Zonas encharcadas", "8.3"),
    "flood": ("Raster → Análisis del terreno → Hidrología → Inundación", "8.3"),
    "ridges": ("Raster → Análisis del terreno → Forma del terreno → Crestas y drenaje", "8.3"),
    "leastcost": ("Raster → Análisis del terreno → Visibilidad y rutas → Ruta de menor coste", "8.3"),
    "piles": ("Raster → Análisis del terreno → Volúmenes → Cubicación de pilas", "8.3"),
    "cutfill": ("Raster → Análisis del terreno → Volúmenes → Desmonte y terraplén", "8.3"),
    "is-valid": ("Vectorial → Geometría → Comprobar validez", "9.4"),
    "fix-topology": ("Vectorial → Geometría → Reparar topología (y panel lateral → Edición → Reparar topología, sobre la capa activa)", "9.4"),
    "simplify": ("Vectorial → Geometría → Simplificar", "9.4"),
    "parallel": ("Vectorial → Geometría → Paralela", "9.4"),
    "fields": ("Vectorial → Tabla → Quitar y renombrar campos (y menú de la cabecera de la tabla de atributos)", "9.5"),
    "make-valid": ("Vectorial → Geometría → Reparar geometrías", "9.4"),
    "buffer": ("Vectorial → Geoprocesos → Búfer", "9.6"),
    "dissolve": ("Vectorial → Geoprocesos → Disolver", "9.6"),
    "union": ("Vectorial → Geoprocesos → Unión", "9.6"),
    "intersect": ("Vectorial → Geoprocesos → Intersección", "9.6"),
    "voronoi": ("Vectorial → Geoprocesos → Voronoi", "9.6"),
    "voronoi-density": ("LiDAR → Calidad → Densidad de Voronoi", "7.3"),
    "dtm-validate": ("LiDAR → Calidad → Validar MDT", "7.3"),
    "canopy-gaps": ("LiDAR → ALS → Huecos del dosel", "7.5"),
    "voxel-metrics": ("LiDAR → ALS → Métricas de vóxeles", "7.5"),
    "trees3d": ("LiDAR → ALS → Segmentar con puntos", "7.5"),
    "tree-metrics": ("LiDAR → ALS → Métricas por árbol", "7.5"),
    "extract": ("Vectorial → Tabla → Extraer valores", "9.5"),
    "join-table": ("Vectorial → Tabla → Unir tabla", "9.5"),
    "merge": ("Vectorial → Geoprocesos → Combinar capas", "9.6"),
    "split": ("Vectorial → Tabla → Dividir por campo", "9.5"),
    "clip-vector": ("Vectorial → Geometría → Recortar capa", "9.4"),
    "near": ("Vectorial → Geoprocesos → Cercanía", "9.6"),
    "summary": ("Archivo → Análisis → EDA → Estadísticas (la misma tabla, agrupada por un campo y con exportación a CSV)", "5"),
    "topology": ("panel lateral → Edición (capas vectoriales) → Comprobar topología", "9.3"),
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
        "La página se genera con `scripts/gen_cli_docs.py` a partir de la definición de las órdenes del propio programa, de modo que lista exactamente lo que admite la versión publicada. Todas las órdenes aceptan además `--help`, que imprime lo mismo en la terminal. Todas admiten también `--force`, que permite que la salida dada con `-o` sustituya a un archivo existente (nunca a una entrada). Cuando una opción no lleva valor por defecto, la herramienta decide por sí misma o no actúa hasta que se le dé un valor.",
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
        "Cuatro órdenes más no procesan datos: tres tratan de los flujos del constructor (capítulo 13) y una abre el servidor MCP. `hipso schema` imprime en JSON la definición de todas las órdenes y opciones de esta página, que es lo que lee el constructor para ofrecer cada herramienta. `hipso run flujo.model.json` ejecuta un flujo guardado sin interfaz (con `--input nombre=ruta` para cambiar una entrada y `--results-dir` para la carpeta de resultados), y `hipso model-validate flujo.model.json` lista lo que impide ejecutarlo. `hipso mcp` arranca un servidor MCP (Protocolo de Contexto de Modelo) por la entrada y la salida estándar: un asistente o cualquier cliente MCP descubre todas las órdenes de esta página como herramientas, con sus opciones tomadas del mismo esquema, y las llama (capítulo 14).", "",
    ]
    (RAIZ / "docs" / "CLI.md").write_text("\n".join(out), encoding="utf-8")
    print(f"{len(esquema)} órdenes, {len(resto)} sin grupo: {resto}")


if __name__ == "__main__":
    main()
