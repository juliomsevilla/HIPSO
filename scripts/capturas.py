#!/usr/bin/env python3
"""Captura una escena de HIPSO con `--window-png` (la ventana entera, sin barra de título ni nada más
de la pantalla), espera a que el archivo exista, cierra el programa y deja la imagen a 2400 px de ancho.

Uso: python3 scripts/capturas.py <salida.png> [--espera SEG] -- <argumentos de hipso-gui>
Necesita que la pantalla esté despierta (caffeinate -d -i).
"""
import os
import subprocess
import sys
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GUI = os.path.join(RAIZ, "target/release/hipso-gui")


def capturar(salida, args, espera=8, limite=900, ancho=2400):
    crudo = salida + ".crudo.png"
    if os.path.exists(crudo):
        os.remove(crudo)
    p = subprocess.Popen([GUI, *args, "--window-png", crudo, "--shot-wait", str(espera)],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    t0 = time.time()
    while time.time() - t0 < limite and not os.path.exists(crudo):
        if p.poll() is not None:
            break
        time.sleep(0.5)
    time.sleep(1.5)
    p.kill()
    p.wait()
    if not os.path.exists(crudo):
        raise SystemExit(f"no se capturó: {salida}")
    subprocess.run(["sips", "--resampleWidth", str(ancho), crudo, "--out", salida], check=True, stdout=subprocess.DEVNULL)
    os.remove(crudo)
    return salida


if __name__ == "__main__":
    a = sys.argv[1:]
    esp = 8
    if "--espera" in a:
        i = a.index("--espera")
        esp = float(a[i + 1])
        del a[i:i + 2]
    i = a.index("--")
    print(capturar(a[0], a[i + 1:], esp))
