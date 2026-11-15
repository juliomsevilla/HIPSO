#!/usr/bin/env python3
"""Comprueba que el capítulo 4 del manual explica todos los botones de la cinta.

Vuelca la cinta con `hipso-gui --dump-ribbon` (pestañas, grupos, botones y menús) y exige que
cada pestaña, cada grupo y cada botón aparezcan en el capítulo 4 de `docs/MANUAL.md`. Así un
botón nuevo sin explicar, o uno que cambia de nombre, hace fallar la comprobación.

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
# la referencia de herramientas debe estar al día con las opciones del programa
antes = (RAIZ / "docs" / "CLI.md").read_text(encoding="utf-8")
subprocess.run([sys.executable, str(RAIZ / "scripts" / "gen_cli_docs.py")], check=True, capture_output=True)
if (RAIZ / "docs" / "CLI.md").read_text(encoding="utf-8") != antes:
    print("docs/CLI.md estaba desfasada: se ha regenerado, hay que incluirla en el commit.")
    sys.exit(1)
print(f"Todos los elementos de la cinta ({len(salida.splitlines())}) están en el capítulo 4.")
