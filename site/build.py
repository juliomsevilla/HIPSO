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


def partir_capitulo_4(lineas):
    """Capítulo 4: la introducción y sus once apartados (4.1 a 4.11)."""
    idx = [i for i, l in enumerate(lineas) if re.match(r"^### 4\.\d+ ", l)]
    return lineas[: idx[0]], [lineas[idx[k]: idx[k + 1] if k + 1 < len(idx) else len(lineas)] for k in range(len(idx))]


def partir_capitulo_7(lineas):
    """Capítulo 7: la introducción y sus tres bloques (7.1, 7.2 y 7.3)."""
    idx = [i for i, l in enumerate(lineas) if re.match(r"^### 7\.\d ", l)]
    return lineas[: idx[0]], [lineas[idx[k]: idx[k + 1] if k + 1 < len(idx) else len(lineas)] for k in range(len(idx))]


# ---------------------------------------------------------------------------------------------
# estructura del sitio

class Pagina:
    def __init__(self, ruta, titulo, grupo, md=None, html_extra="", imagen=None, resumen=""):
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
    "cinta-vectorial": ("10-edicion-vectorial.jpg", "El panel Edición con una capa vectorial."),
    "cinta-flujos": ("09-constructor-de-flujos.jpg", "La pestaña Flujos con el constructor."),
    "vistas": ("06-pnoa-mdt-perfil-3d.jpg", "Planta, perfil y vista 3D con un ráster sobre el perfil."),
    "edicion": ("08-edicion-histograma.jpg", "Edición de puntos con el histograma de elevaciones que acota la reclasificación."),
    "preparar": ("03-cinta-lidar-estadio.jpg", "La pestaña LiDAR con las herramientas de preparación sobre una nube con color."),
    "superficies": ("06-pnoa-mdt-perfil-3d.jpg", "Una hoja del PNOA con su modelo del terreno: la línea del ráster se ve sobre el perfil y como superficie en 3D."),
    "inventario": ("11-modelos-lineales.jpg", "Modelo lineal de una variable de inventario con sus gráficas de diagnóstico."),
    "vectorial": ("10-edicion-vectorial.jpg", "Edición de una capa de rodales sobre la nube y sus herramientas."),
    "raster": ("05-terreno-opengf.jpg", "Un modelo del terreno coloreado por elevación, con su vista 3D."),
    "aba": ("11-modelos-lineales.jpg", "Modelo lineal de una variable de inventario con sus residuos."),
    "flujos": ("09-constructor-de-flujos.jpg", "El constructor de flujos con un inventario por área dibujado como grafo de nodos."),
    "tls": ("07-tls-troncos-y-ramas.jpg", "Troncos y ramas clasificados en una nube de escáner terrestre."),
}


def construir_paginas():
    caps = capitulos_del_manual()
    P = []

    def cap(n, ruta, titulo, grupo, desplazar=1, lineas=None, clave=None):
        md = "\n".join(trocear(lineas if lineas is not None else caps[n], 2, desplazar))
        P.append(Pagina(ruta, titulo, grupo, md=md, imagen=IMAGENES.get(clave or Path(ruta).stem)))

    g = "Empezar"
    cap(1, "manual/que-es.html", "Qué es HIPSO", g)
    cap(2, "manual/instalacion.html", "Instalación", g)
    cap(3, "manual/primeros-pasos.html", "Primeros pasos", g)

    g = "La interfaz"
    intro4, bloques4 = partir_capitulo_4(caps[4])
    paginas4 = [
        ("interfaz", "Las piezas de la ventana", bloques4[0] + bloques4[1]),
        ("cinta-archivo", "Pestaña Archivo", bloques4[2]),
        ("cinta-vista", "Pestaña Vista", bloques4[3]),
        ("cinta-lidar", "Pestaña LiDAR", bloques4[4]),
        ("cinta-vectorial", "Pestaña Vectorial", bloques4[5]),
        ("cinta-raster", "Pestaña Raster", bloques4[6]),
        ("cinta-flujos", "Pestaña Flujos", bloques4[7]),
        ("panel-lateral", "El panel lateral", bloques4[8]),
        ("vistas", "Las vistas", bloques4[9]),
        ("ajustes", "Ajustes", bloques4[10]),
    ]
    for k, (slug, nombre, b) in enumerate(paginas4):
        # el capítulo entero va delante de la primera página como introducción
        lineas = (intro4 if k == 0 else []) + b
        cap(4, f"manual/{slug}.html", nombre, g, desplazar=2 if k else 1, lineas=lineas, clave=slug)

    g = "Nubes de puntos"
    cap(5, "manual/consulta.html", "Herramientas de consulta", g)
    cap(6, "manual/edicion.html", "Edición de nubes de puntos", g)
    intro, bloques = partir_capitulo_7(caps[7])
    cap(7, "manual/lidar.html", "LiDAR: visión general", g, lineas=intro)
    nombres = [("preparar", "Preparar"), ("superficies", "Superficies"), ("inventario", "Inventario de la vegetación")]
    for (slug, nombre), b in zip(nombres, bloques):
        cap(7, f"manual/{slug}.html", f"7. LiDAR · {nombre}", g, desplazar=2, lineas=b, clave=slug)

    g = "Ráster y vectorial"
    cap(8, "manual/raster.html", "Ráster", g)
    cap(9, "manual/vectorial.html", "Vectorial", g)

    g = "Datos y rendimiento"
    cap(10, "manual/formatos.html", "Rutas y formatos", g)
    cap(11, "manual/rendimiento.html", "Rendimiento y memoria", g)
    cap(12, "manual/teselas.html", "Trabajar con miles de teselas", g)

    g = "Flujos y automatización"
    cap(13, "manual/aba.html", "Encadenar un flujo de inventario (ABA)", g, clave="aba")
    P.append(Pagina("flujos-ejemplo.html", "Flujos de ejemplo", g, html_extra=pagina_flujos(), imagen=IMAGENES["flujos"]))
    cap(14, "manual/linea-de-comandos.html", "Línea de comandos", g)
    P.append(Pagina("ordenes.html", "Referencia de herramientas", g, md=(DOCS / "CLI.md").read_text(encoding="utf-8").replace("# Órdenes de `hipso`\n", "# Órdenes de `hipso`\n", 1)))
    P.append(Pagina("recetas.html", "Recetas de clasificación", g, html_extra=pagina_recetas()))

    g = "Ayuda"
    cap(15, "manual/atajos.html", "Atajos", g)
    cap(16, "manual/solucion-de-problemas.html", "Solución de problemas", g)

    g = "Referencia"
    ref = (DOCS / "REFERENCIA.md").read_text(encoding="utf-8").splitlines()
    ref = [l for l in ref]
    P.append(Pagina("referencia.html", "Referencia técnica", g, md="\n".join(trocear(ref, 1, 0))))
    cap(17, "manual/bibliografia.html", "Bibliografía", g)
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

DESTINO_APARTADO = {
    "4.1": "interfaz", "4.2": "interfaz", "4.3": "cinta-archivo", "4.4": "cinta-vista", "4.5": "cinta-lidar",
    "4.6": "cinta-vectorial", "4.7": "cinta-raster", "4.8": "cinta-flujos", "4.9": "panel-lateral",
    "4.10": "vistas", "4.11": "ajustes",
    "6.1": "edicion", "6.2": "edicion", "6.3": "edicion", "6.4": "edicion",
    "7.1": "preparar", "7.2": "superficies", "7.3": "inventario",
}
DESTINO_CAPITULO = {
    "1": "que-es", "2": "instalacion", "3": "primeros-pasos", "4": "interfaz", "5": "consulta", "6": "edicion",
    "7": "lidar", "8": "raster", "9": "vectorial", "10": "formatos", "11": "rendimiento", "12": "teselas",
    "13": "aba", "14": "linea-de-comandos", "15": "atajos", "16": "solucion-de-problemas",
    "17": "bibliografia", "18": "acerca-de",
}


def enlazar_apartados(texto, ruta):
    """Convierte «apartado 7.1» y «capítulo 14» en enlaces a la página que los contiene."""
    base = "" if ruta.startswith("manual/") else "manual/"

    def apartado(m):
        n = m.group(2)
        dest = DESTINO_APARTADO.get(n) or DESTINO_CAPITULO.get(n.split(".")[0])
        return f"[{m.group(1)} {n}]({base}{dest}.html)" if dest else m.group(0)

    def capitulo(m):
        dest = DESTINO_CAPITULO.get(m.group(2))
        return f"[{m.group(1)} {m.group(2)}]({base}{dest}.html)" if dest else m.group(0)

    partes, codigo = [], False
    for linea in texto.split("\n"):
        if linea.startswith("```"):
            codigo = not codigo
        if not codigo and not linea.startswith("#"):
            linea = re.sub(r"\b([Aa]partados?) (\d+\.\d+)\b(?!\.\d)", apartado, linea)
            linea = re.sub(r"\b([Cc]apítulos?) (\d+)\b(?![.\d])", capitulo, linea)
        partes.append(linea)
    return "\n".join(partes)


def a_html(p):
    md = markdown.Markdown(
        extensions=["tables", "fenced_code", "sane_lists", "attr_list", "toc"],
        extension_configs={"toc": {"slugify": lambda v, s: identificador(v), "permalink": "#", "toc_depth": "2-3"}},
    )
    texto = enlazar_apartados(p.md, p.ruta)
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
            c = ' class="activa"' if p is actual else ""
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
    print(f"{len(paginas) + 1} páginas en {SALIDA}")


if __name__ == "__main__":
    main()
