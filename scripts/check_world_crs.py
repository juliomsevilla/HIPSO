#!/usr/bin/env python3
"""Compares hipso-crs with PROJ (pyproj) on points in several countries, without extra grids.

    python3 scripts/check_world_crs.py [max error in metres, default 0.001]

Needs pyproj and `cargo build -p hipso-crs --release --example batch`. Pairs that need a datum
grid PROJ does not have offline are not included: this checks projections and grid-free datum
operations worldwide. Exit status 1 when a pair exceeds the tolerance.
"""
import json, math, os, random, subprocess, sys, tempfile
from pyproj import Transformer

tol = float(sys.argv[1]) if len(sys.argv) > 1 else 0.001
# country, (lon0, lon1, lat0, lat1), src, dst
CASES = [
    ("US NAD83 UTM17N", (-82.0, -79.0, 28.0, 45.0), "EPSG:26917", "EPSG:4269"),
    ("US NAD83 UTM17N", (-82.0, -79.0, 28.0, 45.0), "EPSG:4269", "EPSG:26917"),
    ("US NAD83 UTM17N", (-82.0, -79.0, 28.0, 45.0), "EPSG:26917", "EPSG:4326"),
    ("Canada NAD83(CSRS) UTM17N", (-84.0, -78.0, 43.0, 52.0), "EPSG:2958", "EPSG:4617"),
    ("Canada NAD83(CSRS) UTM17N", (-84.0, -78.0, 43.0, 52.0), "EPSG:4617", "EPSG:2958"),
    ("Portugal PT-TM06", (-9.4, -6.3, 37.0, 42.0), "EPSG:3763", "EPSG:4258"),
    ("Portugal PT-TM06", (-9.4, -6.3, 37.0, 42.0), "EPSG:4258", "EPSG:3763"),
    ("Portugal ETRS89 UTM29N", (-9.4, -6.3, 37.0, 42.0), "EPSG:25829", "EPSG:3763"),
    ("France RGF93 Lambert-93", (-4.0, 8.0, 42.5, 51.0), "EPSG:2154", "EPSG:4171"),
    ("France RGF93 Lambert-93", (-4.0, 8.0, 42.5, 51.0), "EPSG:4326", "EPSG:2154"),
    ("France ETRS89 UTM31N", (0.0, 6.0, 42.5, 51.0), "EPSG:25831", "EPSG:2154"),
    ("Japan JGD2011 plane IX", (138.5, 141.0, 35.0, 40.5), "EPSG:6677", "EPSG:6668"),
    ("Japan JGD2011 plane IX", (138.5, 141.0, 35.0, 40.5), "EPSG:6668", "EPSG:6677"),
    ("Japan JGD2011 plane IX", (138.5, 141.0, 35.0, 40.5), "EPSG:6677", "EPSG:4326"),
    ("Germany ETRS89 UTM32N", (6.0, 12.0, 47.5, 55.0), "EPSG:25832", "EPSG:4326"),
    ("UK OSGB36 (grid-free ballpark/Helmert)", (-5.0, 1.5, 50.0, 58.0), "EPSG:27700", "EPSG:4326"),
    ("Chile SIRGAS-Chile UTM19S", (-72.0, -69.0, -40.0, -20.0), "EPSG:5361", "EPSG:4326"),
    ("Australia GDA94 MGA55", (144.0, 150.0, -40.0, -30.0), "EPSG:28355", "EPSG:4283"),
]
random.seed(11)
def pts(src, box, n=60):
    to_src = Transformer.from_crs("EPSG:4326", src, always_xy=True)
    out = []
    for _ in range(n):
        x, y = to_src.transform(random.uniform(box[0], box[1]), random.uniform(box[2], box[3]))
        if math.isfinite(x) and math.isfinite(y):
            out.append([x, y])
    return out
cases = [{"src": s, "dst": d, "pts": pts(s, b)} for _, b, s, d in CASES]
d = tempfile.mkdtemp()
path = os.path.join(d, "cases.json")
open(path, "w").write(json.dumps(cases))
res = json.loads(subprocess.run(["target/release/examples/batch", path], capture_output=True, text=True, check=True).stdout)
bad_any = False
print(f"{'case':<42}{'src':<12}{'dst':<12}{'pts':>4}{'max err (m)':>13}")
for (name, *_), c, r in zip(CASES, cases, res):
    t = Transformer.from_crs(c["src"], c["dst"], always_xy=True)
    geo = lambda e: e in ("EPSG:4326", "EPSG:4258", "EPSG:4269", "EPSG:4617", "EPSG:4171", "EPSG:6668", "EPSG:4283")
    worst, bad = 0.0, 0
    for p, q in zip(c["pts"], r["res"]):
        if not isinstance(q, list):
            bad += 1
            continue
        ref = t.transform(*p)
        dd = [abs(a - b) for a, b in zip(ref[:2], q[:2])]
        if geo(c["dst"]):
            dd = [x * 111320 for x in dd]
        worst = max(worst, math.hypot(*dd))
    flag = "" if worst <= tol and not bad else "  <-- OVER TOLERANCE" + (f" ({bad} failed)" if bad else "")
    bad_any |= bool(flag)
    print(f"{name:<42}{c['src']:<12}{c['dst']:<12}{len(c['pts']):>4}{worst:>13.6f}{flag}")
sys.exit(1 if bad_any else 0)
