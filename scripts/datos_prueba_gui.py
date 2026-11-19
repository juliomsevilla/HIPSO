#!/usr/bin/env python3
"""Crea un conjunto pequeño de datos sintéticos para probar la interfaz de HIPSO sin los datos reales.

Escribe, en la carpeta que se indique (por defecto `target/datos_prueba`):
  * escena.las: una nube de unos 530 000 puntos en ETRS89 / UTM 30N (EPSG:25830) con terreno ondulado,
    62 árboles de copa cónica, un edificio de tejado plano, dos postes con un cable y ruido, sin clasificar;
  * escena_desplazada.las: la misma escena movida 1,5 m en x y 0,8 m en y y 0,3 m en z, para probar
    comparar y ajustar nubes;
  * rodales.geojson: 12 rodales (polígonos) con especie, altura, edad y cobertura;
  * parcelas.geojson: 30 parcelas (puntos) con volumen, área basal y densidad;
  * parcelas.csv: la tabla de las parcelas con coma decimal y separador de punto y coma;
  * trazo.geojson: una línea que cruza la escena, para el perfil, la ruta y la paralela;
  * topologia_rota.geojson: cuatro polígonos con una ranura, un solape y un hueco, para reparar la topología.

Solo usa la biblioteca estándar. Uso: python3 scripts/datos_prueba_gui.py [carpeta]
"""
import json
import math
import os
import random
import struct
import sys

OX, OY = 440000.0, 4470000.0
SIDE = 200.0


def terrain(x, y):
    return 612.0 + 0.04 * x + 0.02 * y + 6.0 * math.sin(x / 38.0) * math.cos(y / 46.0)


def escena(dx=0.0, dy=0.0, dz=0.0, seed=7):
    rnd = random.Random(seed)
    pts = []  # x, y, z, intensidad, retorno (número, total), tiempo GPS
    # suelo: puntos al azar, con una banda por vuelo
    for _ in range(330_000):
        x, y = rnd.uniform(0, SIDE), rnd.uniform(0, SIDE)
        pts.append((x, y, terrain(x, y) + rnd.gauss(0, 0.03), rnd.randint(20, 90), 1, 1))
    # árboles: copas cónicas con varios retornos
    trees = []
    for _ in range(62):
        tx, ty = rnd.uniform(12, SIDE - 12), rnd.uniform(12, SIDE - 12)
        if 80 < tx < 120 and 80 < ty < 100:  # sin árboles dentro del edificio
            continue
        trees.append((tx, ty, rnd.uniform(7, 19), rnd.uniform(1.6, 3.4)))
    for tx, ty, h, r in trees:
        g = terrain(tx, ty)
        for _ in range(int(2500 * (h / 12))):
            t = rnd.random() ** 0.6
            rad = r * (1 - t) * math.sqrt(rnd.random())
            a = rnd.uniform(0, 2 * math.pi)
            x, y = tx + rad * math.cos(a), ty + rad * math.sin(a)
            z = g + 2.0 + t * (h - 2.0)
            n = rnd.choice((1, 1, 2, 3))
            pts.append((x, y, z + rnd.gauss(0, 0.05), rnd.randint(60, 160), rnd.randint(1, n), n))
        for _ in range(60):  # tronco
            pts.append((tx + rnd.gauss(0, 0.1), ty + rnd.gauss(0, 0.1), g + rnd.uniform(0.2, 2.0), 40, 1, 1))
    # edificio de 40 x 16 m con tejado plano a 8 m
    bx0, by0, bx1, by1 = 80.0, 84.0, 120.0, 100.0
    base = terrain((bx0 + bx1) / 2, (by0 + by1) / 2)
    for _ in range(26_000):
        x, y = rnd.uniform(bx0, bx1), rnd.uniform(by0, by1)
        pts.append((x, y, base + 8.0 + rnd.gauss(0, 0.02), 120, 1, 1))
    for _ in range(5_000):  # paredes
        if rnd.random() < 0.5:
            x, y = rnd.choice((bx0, bx1)), rnd.uniform(by0, by1)
        else:
            x, y = rnd.uniform(bx0, bx1), rnd.choice((by0, by1))
        pts.append((x, y, base + rnd.uniform(0, 8.0), 100, 1, 1))
    # cable entre dos postes de 14 m, con catenaria
    p0, p1 = (20.0, 150.0), (180.0, 160.0)
    for p in (p0, p1):
        g = terrain(*p)
        for _ in range(300):
            pts.append((p[0] + rnd.gauss(0, 0.15), p[1] + rnd.gauss(0, 0.15), g + rnd.uniform(0, 14.0), 80, 1, 1))
    for _ in range(4000):
        t = rnd.random()
        x, y = p0[0] + t * (p1[0] - p0[0]), p0[1] + t * (p1[1] - p0[1])
        z0 = terrain(*p0) + 13.0 + t * (terrain(*p1) - terrain(*p0))
        sag = 4.0 * 4 * t * (1 - t)
        pts.append((x + rnd.gauss(0, 0.04), y + rnd.gauss(0, 0.04), z0 - sag + rnd.gauss(0, 0.04), 70, 1, 1))
    # ruido: aves y puntos bajos
    for _ in range(300):
        x, y = rnd.uniform(0, SIDE), rnd.uniform(0, SIDE)
        pts.append((x, y, terrain(x, y) + rnd.uniform(35, 60), 30, 1, 1))
    for _ in range(200):
        x, y = rnd.uniform(0, SIDE), rnd.uniform(0, SIDE)
        pts.append((x, y, terrain(x, y) - rnd.uniform(3, 12), 10, 1, 1))
    return [(OX + x + dx, OY + y + dy, z + dz, i, r, n) for (x, y, z, i, r, n) in pts]


def escribe_las(path, pts):
    scale = 0.01
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    zs = [p[2] for p in pts]
    off = (math.floor(min(xs)), math.floor(min(ys)), math.floor(min(zs)))
    # VLR con la clave de GeoTIFF del sistema proyectado (EPSG:25830)
    keys = [1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 25830]
    vlr_data = struct.pack("<%dH" % len(keys), *keys)
    vlr = struct.pack("<H16sHH32s", 0, b"LASF_Projection", 34735, len(vlr_data), b"GeoKeyDirectoryTag") + vlr_data
    header_size, rec_len = 227, 28
    offset_to_points = header_size + len(vlr)
    by_return = [0] * 5
    for p in pts:
        if 1 <= p[4] <= 5:
            by_return[p[4] - 1] += 1
    head = struct.pack(
        "<4sHH16sBB32s32sHHHIIBHI5I3d3d6d",
        b"LASF", 0, 0, b"\0" * 16, 1, 2, b"HIPSO datos de prueba".ljust(32, b"\0"), b"datos_prueba_gui.py".ljust(32, b"\0"),
        1, 2026 - 2000 + 2000, header_size, offset_to_points, 1, 1, rec_len, len(pts), *by_return,
        scale, scale, scale, *off,
        max(xs), min(xs), max(ys), min(ys), max(zs), min(zs),
    )
    # el formato de cabecera de LAS 1.2 tiene 227 bytes: se comprueba al escribir
    assert len(head) == header_size, len(head)
    with open(path, "wb") as f:
        f.write(head)
        f.write(vlr)
        buf = bytearray()
        for k, (x, y, z, inten, ret, nret) in enumerate(pts):
            flags = (ret & 7) | ((nret & 7) << 3)
            buf += struct.pack(
                "<iiiHBBbBHd",
                round((x - off[0]) / scale), round((y - off[1]) / scale), round((z - off[2]) / scale),
                inten, flags, 0, 0, 0, 1, k * 1e-5,
            )
            if len(buf) > 4_000_000:
                f.write(buf)
                buf.clear()
        f.write(buf)


def poligono(x0, y0, x1, y1):
    return [[x0, y0], [x0, y1], [x1, y1], [x1, y0], [x0, y0]]


def geojson(path, features):
    crs = {"type": "name", "properties": {"name": "urn:ogc:def:crs:EPSG::25830"}}
    with open(path, "w") as f:
        json.dump({"type": "FeatureCollection", "crs": crs, "features": features}, f)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "target/datos_prueba"
    os.makedirs(out, exist_ok=True)
    pts = escena()
    escribe_las(os.path.join(out, "escena.las"), pts)
    escribe_las(os.path.join(out, "escena_desplazada.las"), escena(1.5, 0.8, 0.3, seed=7))
    print(f"escena.las: {len(pts)} puntos")
    rnd = random.Random(11)
    # rodales: cuadrícula de 4 x 3 sobre la escena
    feats = []
    for k in range(12):
        i, j = k % 4, k // 4
        sp = rnd.choice(["pino", "encina", "eucalipto"])
        alt = rnd.gauss({"pino": 14, "encina": 8, "eucalipto": 20}[sp], 2.0)
        edad = max(8, alt * 3 + rnd.gauss(0, 5))
        cob = min(100, max(10, alt * 3.5 + rnd.gauss(0, 8)))
        w, h = SIDE / 4, SIDE / 3
        feats.append({"type": "Feature", "properties": {"id": k + 1, "especie": sp, "altura": round(alt, 2), "edad": round(edad, 1), "cobertura": round(cob, 1)},
                      "geometry": {"type": "Polygon", "coordinates": [poligono(OX + i * w, OY + j * h, OX + (i + 1) * w, OY + (j + 1) * h)]}})
    geojson(os.path.join(out, "rodales.geojson"), feats)
    # parcelas: 30 puntos y su tabla
    rows, feats = [], []
    for k in range(30):
        x, y = OX + rnd.uniform(10, SIDE - 10), OY + rnd.uniform(10, SIDE - 10)
        alt = rnd.gauss(13, 4)
        ab = max(2.0, alt * 1.7 + rnd.gauss(0, 3))
        vol = max(5.0, ab * alt * 0.38 + rnd.gauss(0, 12))
        n = int(max(100, rnd.gauss(650, 180)))
        feats.append({"type": "Feature", "properties": {"PARCELA": k + 1, "VOL": round(vol, 1), "AB": round(ab, 2), "N": n},
                      "geometry": {"type": "Point", "coordinates": [round(x, 2), round(y, 2)]}})
        rows.append((k + 1, vol, ab, n, alt))
    geojson(os.path.join(out, "parcelas.geojson"), feats)
    with open(os.path.join(out, "parcelas.csv"), "w") as f:
        f.write("PARCELA;VOL;AB;N;ALTURA_DOM\n")
        for r in rows:
            f.write(f"{r[0]};{str(round(r[1], 1)).replace('.', ',')};{str(round(r[2], 2)).replace('.', ',')};{r[3]};{str(round(r[4], 2)).replace('.', ',')}\n")
    geojson(os.path.join(out, "trazo.geojson"), [{"type": "Feature", "properties": {"id": 1},
            "geometry": {"type": "LineString", "coordinates": [[OX + 5, OY + 20], [OX + 70, OY + 60], [OX + 110, OY + 130], [OX + 195, OY + 175]]}}])
    # topología rota: A y B con una ranura de 0,3 m; C pisa a A; un hueco de 2 x 2 m entre D y los demás
    t = []
    for name, ring in [
        ("A", poligono(OX, OY, OX + 50, OY + 50)),
        ("B", poligono(OX + 50.3, OY, OX + 100, OY + 50)),
        ("C", poligono(OX + 40, OY + 30, OX + 70, OY + 70)),
        ("D", poligono(OX + 100.0, OY, OX + 150, OY + 50)),
    ]:
        t.append({"type": "Feature", "properties": {"id": name}, "geometry": {"type": "Polygon", "coordinates": [[[x, y] for x, y in ring]]}})
    geojson(os.path.join(out, "topologia_rota.geojson"), t)
    print("datos escritos en", out)


if __name__ == "__main__":
    main()
