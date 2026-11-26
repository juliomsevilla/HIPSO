#!/usr/bin/env python3
"""Comprueba que el capítulo 4 del manual explica todos los botones de la cinta.

Vuelca la cinta con `hipso-gui --dump-ribbon` (pestañas, grupos, botones y menús) y exige que
cada pestaña, cada grupo y cada botón aparezcan en el capítulo 4 de `docs/MANUAL.md`. Así un
botón nuevo sin explicar, o uno que cambia de nombre, hace fallar la comprobación. Además, cada
herramienta de LiDAR, Ráster y Vectorial debe tener su título en los capítulos 7 a 9.

Uso: python3 scripts/comprobar_manual_cinta.py [ruta de hipso-gui]
"""
import re
import subprocess
import sys
from pathlib import Path

RAIZ = Path(__file__).resolve().parent.parent
gui = sys.argv[1] if len(sys.argv) > 1 else str(RAIZ / "target" / "release" / "hipso-gui")
salida = subprocess.run([gui, "--dump-ribbon"], capture_output=True, text=True, check=True).stdout
manual = (RAIZ / "docs" / "MANUAL.md").read_text(encoding="utf-8")
cap4 = manual[manual.index("## 4. La interfaz"): manual.index("## 5. ")]
faltan = []
for linea in salida.splitlines():
    campos = linea.split("\t")
    tipo, nombre = campos[0], campos[1]
    if nombre and nombre not in cap4:
        faltan.append(f"{tipo}: {nombre}")
if faltan:
    print("Sin explicar en el capítulo 4 del manual:\n  " + "\n  ".join(faltan))
    sys.exit(1)
# y cada herramienta de LiDAR, Ráster y Vectorial tiene además su método en los capítulos 7 a 9:
# un título propio o, en los productos del terreno, una fila de la tabla de su menú
metodos = manual[manual.index("## 7. "): manual.index("## 10. ")]
titulos = {re.sub(r"^#+ (?:\d+\.\d+ )?", "", l) for l in metodos.splitlines() if l.startswith("#")}
filas = {l.split("|")[1].strip() for l in metodos.splitlines() if l.startswith("| ")}
pestana, sin_metodo = "", []
for linea in salida.splitlines():
    tipo, nombre = linea.split("\t")[:2]
    if tipo == "TAB":
        pestana = nombre
    elif pestana in ("LiDAR", "Vectorial", "Ráster") and tipo in ("BIG", "SMALL", "MENUITEM"):
        if nombre not in titulos and nombre not in filas:
            sin_metodo.append(f"{pestana}: {nombre}")
if sin_metodo:
    print("Sin título propio en los capítulos 7 a 9:\n  " + "\n  ".join(sin_metodo))
    sys.exit(1)
# la referencia de herramientas debe estar al día con las opciones del programa
antes = (RAIZ / "docs" / "CLI.md").read_text(encoding="utf-8")
subprocess.run([sys.executable, str(RAIZ / "scripts" / "gen_cli_docs.py")], check=True, capture_output=True)
if (RAIZ / "docs" / "CLI.md").read_text(encoding="utf-8") != antes:
    print("docs/CLI.md estaba desfasada: se ha regenerado, hay que incluirla en el commit.")
    sys.exit(1)
print(f"Todos los elementos de la cinta ({len(salida.splitlines())}) están en el capítulo 4 y las herramientas, en los capítulos 7 a 9.")
