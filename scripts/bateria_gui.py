#!/usr/bin/env python3
"""Batería de pruebas de la interfaz y de las herramientas de HIPSO con datos reales.

Lanza `target/release/hipso-gui` una vez por escenario con los interruptores de arranque del
programa (`--window-png` guarda la ventana entera como PNG pasados unos segundos), vigila que no se
cierre ni entre en pánico, mide el tiempo hasta la captura y la memoria máxima, y escribe un informe.
Después ejecuta con `target/release/hipso` las mismas herramientas que lanzan las ventanas de la
interfaz sobre datos reales y anota si terminan, cuánto tardan y qué dicen.

Uso: python3 scripts/bateria_gui.py [--solo gui|cli] [--filtro texto] [carpeta de datos]
La carpeta de datos por defecto es /Volumes/Ext1Tb/EasyLiDAR_data. Los resultados van a target/bateria.
Solo usa la biblioteca estándar.
"""
import os
import subprocess
import sys
import threading
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GUI = os.path.join(RAIZ, "target/release/hipso-gui")
CLI = os.path.join(RAIZ, "target/release/hipso")
SALIDA = os.path.join(RAIZ, "target/bateria")


def datos():
    return "/Volumes/Ext1Tb/EasyLiDAR_data"


def escenarios(D):
    pnoa = f"{D}/PNOA/LiDAR/PNOA_2023_EXT_655-4371_H29_NPC02.laz"
    pnoa4 = [f"{D}/PNOA/LiDAR/PNOA_2023_EXT_{a}_H29_NPC02.laz" for a in ("655-4371", "655-4372", "656-4371", "656-4372")]
    aba = f"{D}/ABA/LiDAR/PNOA_2010_Lote7_CYL-MAD_434-4540_ORT-CLA-COL.LAZ"
    aba_mdt = f"{D}/ABA/LiDAR/PNOA_2010_Lote7_CYL-MAD_434-4540_ORT-CLA-COL_mdt_1m_tin_clase2.tif"
    ortos = sorted(f for f in os.listdir(f"{D}/PNOA/Raster") if f.endswith(".tif"))
    orto = f"{D}/PNOA/Raster/{ortos[0]}" if ortos else ""
    s = []

    def add(nombre, args, espera=6, limite=240):
        s.append(dict(nombre=nombre, args=args, espera=espera, limite=limite))

    # nubes de puntos
    add("p01_pnoa", [pnoa])
    for c in ("class", "intensity", "return", "hag", "scan-angle", "earth"):
        add(f"p02_pnoa_color_{c}", [pnoa, "--color", c])
    add("p03_pnoa_3d", [pnoa, "--layout", "3d", "--orbit", "30,35"])
    add("p04_pnoa_todo", [pnoa, "--layout", "all", "--orbit", "-40,30"])
    add("p05_pnoa_perfil", [pnoa, "--layout", "plan-profile", "--profile", "655100,4370500,655900,4370500,20"])
    add("p06_pnoa_cuatro_hojas", pnoa4, 10)
    add("p07_pnoa_seleccion_reclasificar", [pnoa, "--select", "655300,4370300,655500,4370500", "--reclass", "7", "--color", "class"])
    add("p08_pnoa_seleccion_borrar", [pnoa, "--select", "655300,4370300,655500,4370500", "--delete"])
    add("p09_pnoa_histograma", [pnoa, "--histogram"])
    add("p10_pnoa_eda", [pnoa, "--charts"], 8)
    add("p11_pnoa_oscuro", [pnoa, "--dark"])
    add("p12_pnoa_claro", [pnoa, "--light", "--side-info"])
    add("p13_pnoa_edicion", [pnoa, "--side-edit"])
    for k, nombre in ((0, "osm"), (1, "pnoa_orto"), (2, "ign_base"), (3, "sentinel_natural"), (4, "sentinel_infrarrojo")):
        add(f"p14_pnoa_mapa_base_{nombre}", [pnoa, "--basemap", str(k)], 14)
    add("p15_copc_autzen", [f"{D}/autzen-classified.copc.laz"], 8)
    add("p16_copc_lion", [f"{D}/lion_takanawa.copc.laz"])
    add("p17_copc_sofi", [f"{D}/sofi.copc.laz"], 25)
    add("p18_dron_Z1_65M", [f"{D}/QULTIVA/Z1_170426.las"], 45, 400)
    add("p19_tls_nogueiro5_5G", [f"{D}/TLS/ATM SUBASTA NOGUEIRO5.las"], 100, 600)
    add("p20_lineas_raildata", [f"{D}/Powerlines/RAILDATA.laz"], 25)
    add("p21_lineas_editada", [f"{D}/Powerlines/20200207_editada.laz"], 8)
    add("p22_kladno_6G", [f"{D}/Kladno station pointcloud.las"], 100, 600)
    dales = sorted(os.listdir(f"{D}/DALES2"))
    if dales:
        add("p23_dales", [f"{D}/DALES2/{dales[0]}", "--color", "class"])
    npc3 = sorted(f for f in os.listdir(f"{D}/NPC03") if not f.startswith("."))
    if npc3:
        add("p24_npc03", [f"{D}/NPC03/{npc3[0]}"])
    add("p25_aba_2010", [aba])
    add("p26_aba_2010_mdt", [aba, aba_mdt, "--layout", "3d"])
    add("p27_muestra_copc_las", [f"{D}/2485_1109.copc-sample.las"], 8)
    # capas vectoriales y tablas
    add("v01_finca", [f"{D}/PNOA/FINCA.geojson"])
    add("v02_galicia2_shp_136M", [f"{D}/galicia2.shp"], 40, 400)
    add("v03_extremadura_gpkg", [f"{D}/extremadura.gpkg"], 25, 400)
    add("v04_galicia_gpkg_431M", [f"{D}/galicia.gpkg"], 60, 600)
    add("v05_aba_parcelas_rodales", [f"{D}/ABA/Campo/Parcelas.shp", f"{D}/ABA/Vectorial/rodales.shp", f"{D}/ABA/Campo/parcelas_metricas.shp", "--table"])
    add("v06_aba_metricas_eda", [f"{D}/ABA/Campo/parcelas_metricas.shp", "--charts", "--table"], 8)
    add("v07_inventario_xlsx_eda", [f"{D}/ABA/Campo/Datos_Inventario.xlsx", "--charts"], 8)
    add("v08_tls_cubicado", [f"{D}/TLS/ATM SUBASTA NOGUEIRO5_cubicado.shp", "--charts", "--table"], 8)
    add("v09_tls_trozas_csv", [f"{D}/TLS/ATM SUBASTA NOGUEIRO5_cubicado_trozas.csv", "--charts"], 8)
    add("v10_galicia2_eda", [f"{D}/galicia2.shp", "--charts"], 45, 400)
    add("v11_vsel_all", [f"{D}/ABA/Campo/parcelas_metricas.shp", "--charts", "--vsel-all"], 8)
    # rásteres
    if orto:
        add("r01_ortofoto", [orto], 40, 300)
    if len(ortos) > 1:
        add("r02_dos_ortofotos", [f"{D}/PNOA/Raster/{ortos[0]}", f"{D}/PNOA/Raster/{ortos[1]}"], 70, 400)
    add("r05_ortofoto_abriendo", [orto, "--basemap", "0"], 3)
    add("r03_mdt_aba", [aba_mdt])
    add("r04_mdt_aba_3d", [aba_mdt, "--layout", "3d", "--exag", "2"])
    # pestañas de la cinta con datos delante
    for t, nombre in enumerate(("archivo", "vista", "lidar", "vectorial", "raster", "flujos")):
        add(f"c0{t}_cinta_{nombre}", [pnoa, f"{D}/PNOA/FINCA.geojson", "--tab", str(t)], 6)
    # ventanas de herramientas
    for a in ("terrain", "obia", "align", "reproject", "rasterops", "vector", "change", "model", "corridor",
              "clitool", "clitool-near", "clitool-fields", "clitool-topology"):
        add(f"w_accion_{a}", [pnoa, f"{D}/PNOA/FINCA.geojson", "--action", a], 5)
    for t in ("classify", "dtm", "chm", "metrics", "trees", "clip", "export", "tls", "vdensity", "intensityimage", "flightlines", "swath"):
        add(f"w_herramienta_{t}", [pnoa, "--tool", t], 5)
    return s


def lanza(esc):
    png = os.path.join(SALIDA, esc["nombre"] + ".png")
    if os.path.exists(png):
        os.remove(png)
    args = [GUI] + esc["args"] + ["--window-png", png, "--shot-wait", str(esc["espera"])]
    t0 = time.time()
    p = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, errors="replace")
    lineas = []
    hecho = threading.Event()

    def lee():
        for l in p.stderr:
            lineas.append(l.rstrip())
            if "ventana guardada" in l or "no se guardó la ventana" in l:
                hecho.set()

    th = threading.Thread(target=lee, daemon=True)
    th.start()
    pico = 0
    estado = "?"
    while True:
        time.sleep(0.5)
        r = subprocess.run(["ps", "-o", "rss=", "-p", str(p.pid)], capture_output=True, text=True).stdout.strip()
        if r:
            pico = max(pico, int(r) // 1024)
        if hecho.is_set():
            estado = "OK" if os.path.exists(png) and os.path.getsize(png) > 20_000 else "CAPTURA VACÍA"
            break
        if p.poll() is not None:
            estado = "SE CERRÓ"
            break
        if time.time() - t0 > esc["limite"]:
            estado = "TIEMPO AGOTADO"
            break
    seg = time.time() - t0
    if p.poll() is None:
        p.terminate()
        try:
            p.wait(5)
        except subprocess.TimeoutExpired:
            p.kill()
    panico = [l for l in lineas if "panicked" in l or "thread '" in l or "RUST_BACKTRACE" in l]
    if panico and estado == "OK":
        estado = "OK CON PÁNICO"
    return dict(nombre=esc["nombre"], estado=estado, seg=seg - esc["espera"], rss=pico,
                png=os.path.getsize(png) if os.path.exists(png) else 0, cola=[l for l in lineas if l][-3:], panico=panico[:3], code=p.returncode)


def comandos(D):
    pnoa = f"{D}/PNOA/LiDAR/PNOA_2023_EXT_655-4371_H29_NPC02.laz"
    aba = f"{D}/ABA/LiDAR/PNOA_2010_Lote7_CYL-MAD_434-4540_ORT-CLA-COL.LAZ"
    o = os.path.join(SALIDA, "salidas")
    c = []

    def add(nombre, args, limite=600):
        c.append(dict(nombre=nombre, args=args, limite=limite))

    add("info_pnoa", ["info", pnoa])
    add("clasificar_pnoa", ["classify", pnoa, "--ground", "csf", "--buildings", "--results-dir", o])
    add("mdt_pnoa", ["dtm", pnoa, "--results-dir", o])
    add("mds_pnoa", ["dsm", pnoa, "--results-dir", o])
    add("chm_pnoa", ["chm", pnoa, "--results-dir", o])
    add("metricas_pnoa", ["metrics", pnoa, "--results-dir", o])
    add("info_aba", ["info", aba])
    add("mdt_aba", ["dtm", aba, "--results-dir", o])
    add("chm_aba", ["chm", aba, "--results-dir", o])
    parcelas = f"{D}/ABA/Campo/parcelas_metricas.shp"
    add("resumen_parcelas", ["summary", parcelas, "--results-dir", o])
    add("campos_parcelas", ["fields", parcelas, "--rename", "PARCELA=parcela_id", "--results-dir", o])
    add("simplificar_rodales", ["simplify", f"{D}/ABA/Vectorial/rodales.shp", "--tolerance", "5", "--results-dir", o])
    add("reparar_rodales", ["fix-topology", f"{D}/ABA/Vectorial/rodales.shp", "--snap", "1", "--max-gap", "100", "--results-dir", o])
    add("reproyectar_rodales_4326", ["reproject", f"{D}/ABA/Vectorial/rodales.shp", "--to", "4326", "-o", os.path.join(o, "rodales_4326.shp")])
    add("reproyectar_finca", ["reproject", f"{D}/PNOA/FINCA.geojson", "--to", "25829", "-o", os.path.join(o, "finca_25829.geojson")])
    add("simplificar_galicia", ["simplify", f"{D}/galicia2.shp", "--tolerance", "10", "--results-dir", o], 900)
    add("reparar_galicia", ["fix-topology", f"{D}/galicia2.shp", "--snap", "0.5", "--results-dir", o], 900)
    add("validar_galicia", ["is-valid", f"{D}/galicia2.shp"], 600)
    add("topologia_rodales", ["topology", f"{D}/ABA/Vectorial/rodales.shp", "--results-dir", o])
    add("catalogo_pnoa", ["catalog", f"{D}/PNOA/LiDAR"], 900)
    add("copc_autzen_info", ["info", f"{D}/autzen-classified.copc.laz"])
    add("lineas_info", ["info", f"{D}/Powerlines/20200207_editada.laz"])
    add("tls_info", ["info", f"{D}/TLS/ATM SUBASTA NOGUEIRO5.las"], 900)
    add("esquema", ["schema"])
    return c


def corre_cli(c):
    t0 = time.time()
    try:
        r = subprocess.run([CLI] + c["args"], capture_output=True, text=True, errors="replace", timeout=c["limite"])
        estado = "OK" if r.returncode == 0 else f"ERROR {r.returncode}"
        cola = (r.stderr.strip() or r.stdout.strip()).splitlines()[-2:]
    except subprocess.TimeoutExpired:
        estado, cola = "TIEMPO AGOTADO", []
    return dict(nombre=c["nombre"], estado=estado, seg=time.time() - t0, cola=cola)


def main():
    args = sys.argv[1:]
    solo, filtro, D = None, "", datos()
    while args:
        a = args.pop(0)
        if a == "--solo":
            solo = args.pop(0)
        elif a == "--filtro":
            filtro = args.pop(0)
        else:
            D = a
    if not os.path.isdir(D):
        sys.exit(f"no está la carpeta de datos {D}")
    os.makedirs(os.path.join(SALIDA, "salidas"), exist_ok=True)
    informe = []
    if solo in (None, "gui"):
        for e in escenarios(D):
            if filtro and not any(f in e["nombre"] for f in filtro.split(",")):
                continue
            r = lanza(e)
            informe.append(("gui", r))
            print(f"{r['estado']:<16} {r['nombre']:<38} {r['seg']:6.1f} s  {r['rss']:6d} MB", flush=True)
            if r["estado"] != "OK":
                print("   ", r["panico"] or r["cola"], flush=True)
    if solo in (None, "cli"):
        for c in comandos(D):
            if filtro and not any(f in c["nombre"] for f in filtro.split(",")):
                continue
            r = corre_cli(c)
            informe.append(("cli", r))
            print(f"{r['estado']:<16} {r['nombre']:<38} {r['seg']:6.1f} s", flush=True)
            if r["estado"] != "OK":
                print("   ", r["cola"], flush=True)
    with open(os.path.join(SALIDA, "informe.md"), "w") as f:
        f.write("# Informe de la batería\n\n| Tipo | Escenario | Estado | Segundos | MB | Nota |\n|---|---|---|---|---|---|\n")
        for tipo, r in informe:
            nota = " / ".join((r.get("panico") or r.get("cola") or [])).replace("|", "/")[:200]
            f.write(f"| {tipo} | {r['nombre']} | {r['estado']} | {r['seg']:.1f} | {r.get('rss', '')} | {nota} |\n")
    malos = [r for _, r in informe if not r["estado"].startswith("OK")]
    print(f"\n{len(informe)} pruebas, {len(malos)} con problemas. Informe: {SALIDA}/informe.md")


if __name__ == "__main__":
    main()
