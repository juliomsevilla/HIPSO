#!/usr/bin/env python3
"""Construye la página web de HIPSO (GitHub Pages) a partir de los documentos del repositorio.

Lee docs/MANUAL.md, docs/REFERENCIA.md, docs/CLI.md y los flujos de ejemplo, reparte el manual en
páginas por capítulo, y escribe en `salida/` el sitio estático con su navegación, su índice de
búsqueda y las imágenes. No hay nada que mantener a mano: el contenido es el de los documentos, de
modo que la web y el manual que viaja con el programa no pueden contradecirse.

Uso: python3 site/build.py [carpeta de salida]    (necesita el paquete `markdown`)
"""
import html
import json
import re
import shutil
import sys
import unicodedata
from pathlib import Path

import markdown

RAIZ = Path(__file__).resolve().parent.parent
SITIO = RAIZ / "site"
DOCS = RAIZ / "docs"
SALIDA = Path(sys.argv[1]) if len(sys.argv) > 1 else RAIZ / "site" / "salida"
REPO = "https://github.com/juliomsevilla/HIPSO"


def version():
    for linea in (RAIZ / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        m = re.match(r'^version = "(.*)"', linea)
        if m:
            return m.group(1)
    return ""


def sin_acentos(t):
    return "".join(c for c in unicodedata.normalize("NFD", t) if unicodedata.category(c) != "Mn")


def identificador(valor, sep="-"):
    t = sin_acentos(valor).lower()
    t = re.sub(r"[^a-z0-9]+", sep, t).strip(sep)
    return t or "seccion"


# ---------------------------------------------------------------------------------------------
# lectura del manual

def trocear(lineas, nivel, desplazar):
    """Cambia el nivel de los encabezados (fuera de los bloques de código) en `desplazar` niveles."""
    out, codigo = [], False
    for l in lineas:
        if l.startswith("```"):
            codigo = not codigo
        m = re.match(r"^(#{1,6}) ", l)
        if m and not codigo:
            n = max(1, len(m.group(1)) - desplazar)
            l = "#" * n + l[len(m.group(1)):]
        out.append(l)
    return out


def capitulos_del_manual():
    """Devuelve {número de capítulo: líneas} (el capítulo 7 se reparte aparte)."""
    lineas = (DOCS / "MANUAL.md").read_text(encoding="utf-8").splitlines()
    marcas = [(i, re.match(r"^## (\d+)\. ", l)) for i, l in enumerate(lineas)]
    marcas = [(i, int(m.group(1))) for i, m in marcas if m]
    caps = {}
    for k, (i, n) in enumerate(marcas):
        fin = marcas[k + 1][0] if k + 1 < len(marcas) else len(lineas)
        caps[n] = lineas[i:fin]
    return caps


def partir(lineas, n):
    """Un capítulo partido en su introducción y sus apartados (### n.1, ### n.2…)."""
    idx = [i for i, l in enumerate(lineas) if re.match(rf"^### {n}\.\d+ ", l)]
    return lineas[: idx[0]], [lineas[i: j] for i, j in zip(idx, idx[1:] + [len(lineas)])]


# ---------------------------------------------------------------------------------------------
# estructura del sitio

class Pagina:
    def __init__(self, ruta, titulo, grupo, md=None, html_extra="", imagen=None, resumen="", nivel=1):
        self.nivel = nivel          # 2: subpágina, sangrada en el menú
        self.ruta = ruta            # relativa a la salida, p. ej. "manual/interfaz.html"
        self.titulo = titulo
        self.grupo = grupo
        self.md = md
        self.html_extra = html_extra
        self.imagen = imagen        # (archivo, pie)
        self.resumen = resumen
        self.cuerpo = ""
        self.toc = []               # (nivel, id, texto)


IMAGENES = {
    "que-es": ("01-dales-planta-perfil-3d.jpg", "Una nube urbana clasificada (DALES) en planta, perfil y 3D."),
    "interfaz": ("01-dales-planta-perfil-3d.jpg", "La ventana de HIPSO con la vista en planta, el perfil y la vista 3D de una misma nube."),
    "cinta-lidar": ("03-cinta-lidar-estadio.jpg", "La pestaña LiDAR con sus seis grupos."),
    "cinta-flujos": ("09-constructor-de-flujos.jpg", "La pestaña Flujos con el constructor."),
    "vistas": ("06-pnoa-mdt-perfil-3d.jpg", "Planta, perfil y vista 3D con un ráster sobre el perfil."),
    "edicion": ("08-edicion-histograma.jpg", "Edición de puntos con el histograma de elevaciones que acota la reclasificación."),
    "preparar": ("03-cinta-lidar-estadio.jpg", "La pestaña LiDAR con las herramientas de preparación sobre una nube con color."),
    "clasificar": ("04-linea-electrica-clasificada.jpg", "Una línea eléctrica clasificada: suelo, vegetación por estratos, cables y torres."),
    "superficies": ("06-pnoa-mdt-perfil-3d.jpg", "Una hoja del PNOA con su modelo del terreno: la línea del ráster se ve sobre el perfil y como superficie en 3D."),
    "als": ("11-modelos-lineales.jpg", "Modelo lineal de una variable de inventario con sus gráficas de diagnóstico."),
    "tls": ("07-tls-troncos-y-ramas.jpg", "Troncos y ramas clasificados en una nube de escáner terrestre."),
    "raster-terreno": ("05-terreno-opengf.jpg", "Un modelo del terreno coloreado por elevación, con su vista 3D."),
    "vectorial-edicion": ("10-edicion-vectorial.jpg", "Edición de una capa de rodales sobre la nube y sus herramientas."),
    "flujos": ("09-constructor-de-flujos.jpg", "El constructor de flujos con un inventario por área dibujado como grafo de nodos."),
    "asistente": ("12-asistente-ia.jpg", "El panel del asistente de IA junto a una nube clasificada: explica las herramientas y crea flujos."),
}

# las páginas en que se reparten los capítulos 7, 8 y 9: (página, título en el menú) por apartado
APARTADOS = {
    7: [("preparar", "Preparar"), ("clasificar", "Clasificar"), ("calidad", "Calidad"), ("superficies", "Superficies"),
        ("als", "ALS: inventario aéreo"), ("tls", "TLS: escáner terrestre"), ("infraestructuras", "Infraestructuras")],
    8: [("raster-imagen", "Imagen"), ("raster-geoprocesos", "Geoprocesos"), ("raster-terreno", "Análisis del terreno")],
    9: [("vectorial-capas", "Capas"), ("tabla-de-atributos", "Tabla de atributos"), ("vectorial-edicion", "Edición de elementos"),
        ("vectorial-geometria", "Geometría"), ("vectorial-tabla", "Tabla"), ("vectorial-geoprocesos", "Geoprocesos")],
}

# apartado o capítulo del manual -> página; se rellena al construir las páginas
DESTINOS = {}
# direcciones de versiones anteriores de la web que siguen enlazadas desde fuera
REDIRECCIONES = {"manual/inventario.html": "als.html", "manual/aba.html": "flujos.html"}


def indice_de_herramientas(n, bloques):
    """El índice de un capítulo de herramientas: sus apartados y los títulos de cada uno."""
    out = ["", "## Todas las herramientas del capítulo {#indice}", ""]
    for (slug, nombre), b in zip(APARTADOS[n], bloques):
        titulos = [l[5:].strip() for l in b if l.startswith("#### ")]
        enlaces = " · ".join(f"[{t}]({slug}.html#{identificador(t)})" for t in titulos)
        out.append(f"- **[{nombre}]({slug}.html)**: {enlaces}")
    return out


def paginas_de_ordenes():
    """La referencia de herramientas: un índice y una página por área."""
    lineas = (DOCS / "CLI.md").read_text(encoding="utf-8").splitlines()
    marcas = [i for i, l in enumerate(lineas) if l.startswith("## ")]
    intro = lineas[: marcas[0]]
    areas = []
    for i, j in zip(marcas, marcas[1:] + [len(lineas)]):
        titulo = lineas[i][3:].strip()
        areas.append((f"ordenes-{identificador(titulo)}.html", titulo, lineas[i:j]))
    indice = intro + ["", "## Índice por áreas {#indice}", ""]
    for ruta, titulo, b in areas:
        ordenes = [l[4:].strip() for l in b if l.startswith("### ")]
        if not ordenes:
            indice += [f"### [{titulo}]({ruta})", ""]
            continue
        indice += [f"### [{titulo}]({ruta})", "", "| Orden | Dónde está en la interfaz |", "|---|---|"]
        for o in ordenes:
            k = b.index(f"### {o}")
            donde = next(l for l in b[k:] if l.startswith("**En la interfaz:**"))
            donde = donde.removeprefix("**En la interfaz:** ").split(" El método se explica")[0].rstrip(".")
            indice.append(f"| [`{o}`]({ruta}#{identificador(o)}) | {donde} |")
        indice.append("")
    P = [Pagina("ordenes.html", "Referencia de herramientas", "Referencia", md="\n".join(indice))]
    for ruta, titulo, b in areas:
        P.append(Pagina(ruta, titulo, "Referencia", md="\n".join(trocear(b, 2, 1)), nivel=2))
    return P


def construir_paginas():
    caps = capitulos_del_manual()
    P = []

    def cap(n, ruta, titulo, grupo, desplazar=1, lineas=None, apartado=None):
        md = "\n".join(trocear(lineas if lineas is not None else caps[n], 2, desplazar))
        P.append(Pagina(ruta, titulo, grupo, md=md, imagen=IMAGENES.get(Path(ruta).stem)))
        DESTINOS.setdefault(apartado or str(n), ruta)

    g = "Empezar"
    cap(1, "manual/que-es.html", "Qué es HIPSO", g)
    cap(2, "manual/instalacion.html", "Instalación", g)
    cap(3, "manual/primeros-pasos.html", "Primeros pasos", g)

    g = "La interfaz"
    intro4, bloques4 = partir(caps[4], 4)
    paginas4 = [
        ("interfaz", "Las piezas de la ventana", [1, 2]),
        ("cinta-archivo", "Pestaña Archivo", [3]),
        ("cinta-vista", "Pestaña Vista", [4]),
        ("cinta-lidar", "Pestaña LiDAR", [5]),
        ("cinta-vectorial", "Pestaña Vectorial", [6]),
        ("cinta-raster", "Pestaña Raster", [7]),
        ("cinta-flujos", "Pestaña Flujos", [8]),
        ("panel-lateral", "El panel lateral", [9]),
        ("vistas", "Las vistas", [10]),
        ("ajustes", "Ajustes", [11]),
        ("asistente", "El asistente de IA", [12]),
    ]
    for k, (slug, nombre, nums) in enumerate(paginas4):
        # el capítulo entero va delante de la primera página como introducción
        lineas = (intro4 if k == 0 else []) + [l for a in nums for l in bloques4[a - 1]]
        cap(4, f"manual/{slug}.html", nombre, g, desplazar=2 if k else 1, lineas=lineas, apartado=f"4.{nums[0]}")
        for a in nums:
            DESTINOS[f"4.{a}"] = f"manual/{slug}.html"
    DESTINOS["4"] = "manual/interfaz.html"

    g = "Consultar y editar"
    cap(5, "manual/consulta.html", "Herramientas de consulta", g)
    cap(6, "manual/edicion.html", "Edición de nubes de puntos", g)
    DESTINOS.update({"6.1": "manual/edicion.html", "6.2": "manual/edicion.html"})

    for n, grupo, slug in ((7, "LiDAR", "lidar"), (8, "Ráster", "raster"), (9, "Vectorial", "vectorial")):
        intro, bloques = partir(caps[n], n)
        cap(n, f"manual/{slug}.html", "Visión general", grupo, lineas=intro + indice_de_herramientas(n, bloques))
        for k, ((sl, nombre), b) in enumerate(zip(APARTADOS[n], bloques), 1):
            cap(n, f"manual/{sl}.html", nombre, grupo, desplazar=2, lineas=b, apartado=f"{n}.{k}")

    g = "Flujos y automatización"
    cap(13, "manual/flujos.html", "El constructor de flujos", g)
    P.append(Pagina("flujos-ejemplo.html", "Flujos de ejemplo", g, html_extra=pagina_flujos(), imagen=IMAGENES["flujos"]))
    P.append(Pagina("recetas.html", "Recetas de clasificación", g, html_extra=pagina_recetas()))
    cap(14, "manual/linea-de-comandos.html", "Línea de comandos", g)
    cap(12, "manual/teselas.html", "Trabajar con miles de teselas", g)

    g = "Datos y rendimiento"
    cap(10, "manual/formatos.html", "Rutas y formatos", g)
    cap(11, "manual/rendimiento.html", "Rendimiento y memoria", g)

    P.extend(paginas_de_ordenes())
    ref = (DOCS / "REFERENCIA.md").read_text(encoding="utf-8").splitlines()
    P.append(Pagina("referencia.html", "Referencia técnica", "Referencia", md="\n".join(ref)))
    cap(17, "manual/bibliografia.html", "Bibliografía", "Referencia")

    g = "Ayuda"
    cap(15, "manual/atajos.html", "Atajos", g)
    cap(16, "manual/solucion-de-problemas.html", "Solución de problemas", g)
    cap(18, "manual/acerca-de.html", "Acerca de", g)
    return P


DESCRIPCION_FLUJOS = {
    "mdt-chm-arboles": ("MDT, CHM y árboles individuales", "El flujo más corto que produce algo útil: del terreno de una nube se pasa al modelo de alturas de la vegetación y de este a los árboles individuales (cimas, copas y polígonos)."),
    "inventario-por-area": ("Inventario por área (ABA)", "Reúne los pasos del capítulo del inventario por área: el modelo digital del terreno, las métricas LiDAR de cada parcela y la regresión contra el inventario de campo, con su validación cruzada."),
    "cambio-entre-fechas": ("Cambio entre dos fechas", "Calcula el modelo de alturas de dos coberturas y los compara: diferencia de altura, crecimiento, pérdida de cobertura y manchas de dosel perdido."),
    "acopios": ("Volumen de acopios", "A partir de la superficie de un emplazamiento y de los contornos de las pilas, mide el volumen, el área y la altura de cada una sobre una base ajustada a su contorno."),
    "rodales-estadistica": ("Estadística por rodal", "Resume un modelo de alturas dentro de cada rodal (estadística zonal) y agrupa el resultado en una tabla de resumen, lista para un informe."),
    "terreno-derivados": ("Derivados del terreno", "Del modelo digital del terreno de una nube salen la pendiente, la rugosidad, el sombreado y las curvas de nivel en una sola ejecución."),
}


def pagina_flujos():
    partes = [
        "<h1>Flujos de ejemplo</h1>",
        "<p>Un flujo es un grafo de herramientas guardado en un archivo <code>.model.json</code>. Se abre en la pestaña <em>Flujos</em> (<em>Abrir</em>, o el menú de ejemplos de la cinta) y se ejecuta sin cambios sobre los archivos propios, con solo sustituir los nodos de entrada. Los seis flujos que siguen vienen en el repositorio, y cada uno se descarga tal cual para abrirlo en el constructor. Fuera de la interfaz, el mismo archivo se ejecuta con <code>hipso run flujo.model.json --input Nube=hoja.laz</code>.</p>",
    ]
    destino = SALIDA / "flujos"
    destino.mkdir(parents=True, exist_ok=True)
    for f in sorted((DOCS / "recetas" / "flujos").glob("*.model.json")):
        clave = f.name.replace(".model.json", "")
        titulo, texto = DESCRIPCION_FLUJOS.get(clave, (clave, ""))
        d = json.loads(f.read_text(encoding="utf-8"))
        cadena = " → ".join(n.get("label", "") for n in d["nodes"])
        entradas = [n["label"] for n in d["nodes"] if n.get("type") == "input"]
        herramientas = [n["tool"] for n in d["nodes"] if n.get("type") == "tool"]
        shutil.copy(f, destino / f.name)
        partes.append(
            f'<section class="flujo"><h2 id="{clave}">{html.escape(titulo)}</h2><p>{html.escape(texto)}</p>'
            f'<p class="meta">Entradas: {html.escape(", ".join(entradas))}. Órdenes: <code>{html.escape("</code>, <code>".join(herramientas)).replace("&lt;/code&gt;, &lt;code&gt;", "</code>, <code>")}</code>.</p>'
            f'<p><a class="boton" href="flujos/{f.name}" download>Descargar {f.name}</a></p></section>'
        )
    return "\n".join(partes)


def pagina_recetas():
    partes = [
        "<h1>Recetas de clasificación</h1>",
        "<p>Una receta es un archivo de opciones para <code>hipso classify</code> que se pasa con <code>@receta.txt</code>: cada línea es una opción de la orden y las que empiezan por <code>#</code> son comentarios. Las dos que siguen se usan en producción con hojas del PNOA y con líneas eléctricas.</p>",
    ]
    destino = SALIDA / "recetas"
    destino.mkdir(parents=True, exist_ok=True)
    for f in sorted((DOCS / "recetas").glob("*.txt")):
        shutil.copy(f, destino / f.name)
        titulo = f.stem.replace("_", " ").capitalize()
        partes.append(
            f'<section class="flujo"><h2 id="{identificador(f.stem)}">{html.escape(titulo)}</h2>'
            f'<pre><code>{html.escape(f.read_text(encoding="utf-8"))}</code></pre>'
            f'<p><a class="boton" href="recetas/{f.name}" download>Descargar {f.name}</a></p></section>'
        )
    return "\n".join(partes)


# ---------------------------------------------------------------------------------------------
# conversión y plantilla

def enlazar_apartados(texto, ruta):
    """Convierte «apartado 7.1, *Adelgazar nube*» y «capítulo 14» en enlaces a su página y título."""
    def enlace(destino):
        return "../" * ruta.count("/") + destino

    def apartado(m):
        n, titulo = m.group(2), m.group(3)
        dest = DESTINOS.get(n) or DESTINOS.get(n.split(".")[0])
        if not dest:
            return m.group(0)
        out = f"[{m.group(1)} {n}]({enlace(dest)})"
        if titulo:
            # el título enlaza a su encabezado; el primero de la página no lleva ancla
            # un capítulo entero no lleva ancla: su título es el de la página
            ancla = f"#{identificador(titulo)}" if "." in n else ""
            out += f", [*{titulo}*]({enlace(dest)}{ancla})"
        return out

    partes, codigo = [], False
    for linea in texto.split("\n"):
        if linea.startswith("```"):
            codigo = not codigo
        if not codigo and not linea.startswith("#"):
            linea = re.sub(r"\b([Aa]partados?|[Cc]apítulos?) (\d+(?:\.\d+)?)\b(?!\.\d)(?:, \*([^*\n]+)\*)?", apartado, linea)
        partes.append(linea)
    return "\n".join(partes)


def a_html(p):
    md = markdown.Markdown(
        extensions=["tables", "fenced_code", "sane_lists", "attr_list", "toc"],
        extension_configs={"toc": {"slugify": lambda v, s: identificador(v), "permalink": "#", "toc_depth": "2-3"}},
    )
    texto = enlazar_apartados(p.md, p.ruta)
    # enlaces relativos a archivos del repositorio (en docs/ o en la raíz): a su página en GitHub
    def a_repo(m):
        u = m.group(2)
        if u.startswith("../"):
            u = u[3:]
        elif u.endswith(".md") and "/" not in u:
            u = "docs/" + u
        return f"{m.group(1)}({REPO}/blob/main/{u})"
    texto = re.sub(r"(\[[^\]]+\])\(((?:\.\./)?[A-Za-z0-9_./-]+\.md)\)", a_repo, texto)
    # el manual no deja siempre una línea en blanco antes de una lista
    texto = re.sub(r"([^\n])\n(- |\d+\. )", r"\1\n\n\2", texto)
    cuerpo = md.convert(texto)
    p.cuerpo = cuerpo
    p.toc = [(t["level"], t["id"], t["name"]) for t in flatten(md.toc_tokens)]


def flatten(tokens):
    for t in tokens:
        yield t
        yield from flatten(t.get("children", []))


def raiz(ruta):
    return "../" * ruta.count("/")


def menu(paginas, actual):
    r = raiz(actual.ruta)
    grupos = []
    for p in paginas:
        if not grupos or grupos[-1][0] != p.grupo:
            grupos.append((p.grupo, []))
        grupos[-1][1].append(p)
    out = [f'<a class="inicio-enlace{" activa" if actual.ruta == "index.html" else ""}" href="{r}index.html">Inicio</a>']
    for nombre, ps in grupos:
        abierto = " open" if any(p is actual for p in ps) else ""
        out.append(f"<details{abierto}><summary>{html.escape(nombre)}</summary>")
        for p in ps:
            clases = " ".join(x for x, si in (("activa", p is actual), ("sub", p.nivel > 1)) if si)
            c = f' class="{clases}"' if clases else ""
            out.append(f'<a{c} href="{r}{p.ruta}">{html.escape(p.titulo)}</a>')
        out.append("</details>")
    return "\n".join(out)


def pagina_html(plantilla, paginas, p, v, cuerpo=None, es_inicio=False):
    r = raiz(p.ruta)
    ind = paginas.index(p) if p in paginas else -1
    ant = paginas[ind - 1] if ind > 0 else None
    sig = paginas[ind + 1] if 0 <= ind < len(paginas) - 1 else None
    if cuerpo is None:
        cuerpo = p.cuerpo
        if p.imagen:
            f, pie = p.imagen
            cuerpo = cuerpo.replace("</h1>", f'</h1><figure class="captura"><img loading="lazy" src="{r}img/{f}" alt="{html.escape(pie)}"><figcaption>{html.escape(pie)}</figcaption></figure>', 1)
    toc = ""
    if p.toc:
        items = "".join(f'<a class="n{n}" href="#{i}">{html.escape(t)}</a>' for n, i, t in p.toc)
        toc = f'<nav class="toc"><strong>En esta página</strong>{items}</nav>'
    pie = ""
    if ant or sig:
        a = f'<a class="ant" href="{r}{ant.ruta}"><span>Anterior</span>{html.escape(ant.titulo)}</a>' if ant else "<span></span>"
        s = f'<a class="sig" href="{r}{sig.ruta}"><span>Siguiente</span>{html.escape(sig.titulo)}</a>' if sig else "<span></span>"
        pie = f'<nav class="pasos">{a}{s}</nav>'
    editar = f'<a href="{REPO}/blob/main/docs/MANUAL.md">Editar en GitHub</a>' if p.ruta.startswith("manual/") else ""
    return (
        plantilla.replace("{{titulo}}", html.escape(p.titulo))
        .replace("{{raiz}}", r)
        .replace("{{menu}}", menu(paginas, p))
        .replace("{{cuerpo}}", cuerpo)
        .replace("{{toc}}", toc)
        .replace("{{pasos}}", pie)
        .replace("{{version}}", v)
        .replace("{{repo}}", REPO)
        .replace("{{editar}}", editar)
        .replace("{{clase}}", "inicio" if es_inicio else "doc")
    )


def indice_busqueda(paginas, inicio_texto):
    """Un registro por sección (h1, h2 y h3): título, enlace y texto plano."""
    reg = []
    for p in paginas:
        partes = re.split(r'(<h[123][^>]*id="[^"]+"[^>]*>.*?</h[123]>)', p.cuerpo or p.html_extra, flags=re.S)
        titulo, ancla, texto = p.titulo, "", []
        def cerrar():
            t = re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", " ", " ".join(texto)))).strip()
            if t or ancla == "":
                reg.append({"t": titulo, "p": p.titulo, "u": p.ruta + ("#" + ancla if ancla else ""), "x": t[:1200]})
        for fragmento in partes:
            m = re.match(r'<h[123][^>]*id="([^"]+)"[^>]*>(.*?)</h[123]>', fragmento, flags=re.S)
            if m:
                cerrar()
                texto = []
                ancla = m.group(1)
                titulo = re.sub(r"<[^>]+>", "", html.unescape(m.group(2))).replace("¶", "").replace("#", "").strip()
            else:
                texto.append(fragmento)
        cerrar()
    reg.append({"t": "Inicio", "p": "Inicio", "u": "index.html", "x": inicio_texto})
    return reg


def enlaces_rotos():
    """Enlaces internos del sitio construido que apuntan a una página o a un ancla que no existen."""
    anclas = {f.resolve(): set(re.findall(r'id="([^"]+)"', f.read_text(encoding="utf-8"))) for f in SALIDA.rglob("*.html")}
    rotos = []
    for f in SALIDA.rglob("*.html"):
        for u in re.findall(r'(?:href|src)="([^"]+)"', f.read_text(encoding="utf-8")):
            if re.match(r"^(https?:|mailto:|data:)", u):
                continue
            ruta, _, ancla = u.partition("#")
            destino = (f.parent / ruta).resolve() if ruta else f.resolve()
            if not destino.exists() or (ancla and ancla not in anclas.get(destino, set())):
                rotos.append(f"{f.relative_to(SALIDA)} -> {u}")
    return rotos


def main():
    v = version()
    if SALIDA.exists():
        shutil.rmtree(SALIDA)
    SALIDA.mkdir(parents=True)
    paginas = construir_paginas()
    for p in paginas:
        if p.md is not None:
            a_html(p)
        else:
            p.cuerpo = p.html_extra
            p.toc = [(2, m.group(1), re.sub(r"<[^>]+>", "", m.group(2))) for m in re.finditer(r'<h2 id="([^"]+)">(.*?)</h2>', p.html_extra)]
    plantilla = (SITIO / "plantilla.html").read_text(encoding="utf-8")
    for p in paginas:
        destino = SALIDA / p.ruta
        destino.parent.mkdir(parents=True, exist_ok=True)
        destino.write_text(pagina_html(plantilla, paginas, p, v), encoding="utf-8")
    inicio = (SITIO / "contenido" / "inicio.html").read_text(encoding="utf-8").replace("{{version}}", v).replace("{{repo}}", REPO)
    portada = Pagina("index.html", "HIPSO", "")
    (SALIDA / "index.html").write_text(pagina_html(plantilla, paginas, portada, v, cuerpo=inicio, es_inicio=True), encoding="utf-8")
    for vieja, nueva in REDIRECCIONES.items():
        (SALIDA / vieja).write_text(
            f'<!doctype html><meta charset="utf-8"><title>HIPSO</title><meta http-equiv="refresh" content="0; url={nueva}"><a href="{nueva}">La página ha cambiado de dirección</a>',
            encoding="utf-8")
    # estáticos, imágenes y marca
    shutil.copytree(SITIO / "static", SALIDA / "static")
    (SALIDA / "img").mkdir()
    for f in (RAIZ / "assets" / "social").iterdir():
        if f.suffix.lower() in (".jpg", ".png"):
            shutil.copy(f, SALIDA / "img" / f.name)
    for f in (RAIZ / "assets" / "brand").glob("*.svg"):
        shutil.copy(f, SALIDA / "img" / f.name)
    shutil.copy(RAIZ / "assets" / "icon.svg", SALIDA / "img" / "icon.svg")
    plano = re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", inicio))
    (SALIDA / "static" / "busqueda.json").write_text(json.dumps(indice_busqueda(paginas, plano), ensure_ascii=False), encoding="utf-8")
    (SALIDA / ".nojekyll").write_text("", encoding="utf-8")
    rotos = enlaces_rotos()
    if rotos:
        sys.exit("Enlaces rotos:\n  " + "\n  ".join(rotos))
    print(f"{len(paginas) + 1} páginas en {SALIDA}")


if __name__ == "__main__":
    main()
