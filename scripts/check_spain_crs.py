#!/usr/bin/env python3
"""Compares rustlidar-crs with PROJ (pyproj) on points over Spain, with the IGN/ICGC grids.

    python3 scripts/check_spain_crs.py [dir with the grids]   (default: crates/crs/data/grids)

Needs pyproj and a built `cargo build -p rustlidar-crs --release --example batch`.
"""
import json, math, subprocess, sys, os, random
import pyproj
from pyproj import Transformer

grids = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "crates/crs/data/grids")
pyproj.datadir.append_data_dir(grids)
os.environ["PROJ_DATA"] = grids

# (lon0, lon1, lat0, lat1) where each system is used in Spain
AREAS = {
    "peninsula_w": (-9.3, -6.0, 36.0, 43.8), "peninsula_c": (-6.0, 0.0, 36.0, 43.8),
    "peninsula_e": (0.0, 3.4, 38.5, 42.9), "baleares": (1.2, 4.4, 38.6, 40.2),
    "canarias": (-18.2, -13.3, 27.6, 29.5), "cataluna": (0.2, 3.3, 40.5, 42.9),
}
PAIRS = [  # src, dst, areas
    ("EPSG:25830", "EPSG:25829", ["peninsula_w", "peninsula_c"]),
    ("EPSG:25830", "EPSG:25831", ["peninsula_c", "peninsula_e"]),
    ("EPSG:25829", "EPSG:25830", ["peninsula_w", "peninsula_c"]),
    ("EPSG:25831", "EPSG:25830", ["peninsula_e", "cataluna", "baleares"]),
    ("EPSG:25828", "EPSG:25830", ["canarias"]),
    ("EPSG:23029", "EPSG:25829", ["peninsula_w"]), ("EPSG:23030", "EPSG:25830", ["peninsula_c"]),
    ("EPSG:23031", "EPSG:25831", ["peninsula_e", "cataluna", "baleares"]),
    ("EPSG:23030", "EPSG:25829", ["peninsula_c"]), ("EPSG:23029", "EPSG:25830", ["peninsula_w", "peninsula_c"]),
    ("EPSG:23030", "EPSG:25831", ["peninsula_c", "peninsula_e"]),
    ("EPSG:25830", "EPSG:23030", ["peninsula_c"]), ("EPSG:25831", "EPSG:23031", ["cataluna", "baleares"]),
    ("EPSG:4230", "EPSG:4258", ["peninsula_c", "cataluna", "baleares"]),
    ("EPSG:4258", "EPSG:4230", ["peninsula_c"]),
    ("EPSG:4326", "EPSG:25830", ["peninsula_c"]), ("EPSG:32630", "EPSG:25830", ["peninsula_c"]),
    ("EPSG:25830", "EPSG:4326", ["peninsula_c"]), ("EPSG:25830", "EPSG:3857", ["peninsula_c"]),
    ("EPSG:4326", "EPSG:25828", ["canarias"]), ("EPSG:4083", "EPSG:25828", ["canarias"]),
    ("EPSG:4082", "EPSG:25828", ["canarias"]), ("EPSG:25828", "EPSG:4083", ["canarias"]),
    ("EPSG:23028", "EPSG:25828", ["canarias"]),
    ("EPSG:25830", "EPSG:25830", ["peninsula_c"]),
    ("EPSG:3042", "EPSG:25830", ["peninsula_c"]), ("EPSG:2062", "EPSG:25830", ["peninsula_c"]),
    ("EPSG:4258", "EPSG:25830", ["peninsula_c"]), ("EPSG:4258", "EPSG:25829", ["peninsula_w"]),
]
HEIGHT_PAIRS = [  # orthometric (REDNAP EPSG:5782 / Canarias EPSG:9397?) <-> ellipsoidal
    ("EPSG:25830+5782", "EPSG:4979", ["peninsula_c"]), ("EPSG:4979", "EPSG:25830+5782", ["peninsula_c"]),
    ("EPSG:25829+5782", "EPSG:25830+5782", ["peninsula_w"]),
]
random.seed(7)
def pts_for(src, areas, n=60):
    to_src = Transformer.from_crs("EPSG:4326", src, always_xy=True)
    out = []
    for a in areas:
        x0, x1, y0, y1 = AREAS[a]
        for _ in range(n):
            lon, lat = random.uniform(x0, x1), random.uniform(y0, y1)
            x, y = to_src.transform(lon, lat)
            if math.isfinite(x): out.append([x, y])
    return out
cases = [{"src": s, "dst": d, "pts": pts_for(s, a)} for s, d, a in PAIRS]
for s, d, a in HEIGHT_PAIRS:
    c = {"src": s, "dst": d, "pts": [p + [random.uniform(0, 2000)] for p in pts_for(s.split("+")[0], a, 30)]}
    cases.append(c)
open("/tmp/_crs_cases.json", "w").write(json.dumps(cases))
exe = "target/release/examples/batch"
res = json.loads(subprocess.run([exe, "/tmp/_crs_cases.json"], capture_output=True, text=True, check=True).stdout)
worst_all = 0
print(f"{'src':<18}{'dst':<18}{'pts':>5}{'max err (m)':>14}  {'PROJ op'}")
for c, r in zip(cases, res):
    t = Transformer.from_crs(c["src"], c["dst"], always_xy=True)
    geo = lambda e: e in ("EPSG:4326", "EPSG:4258", "EPSG:4230", "EPSG:4979")
    worst, bad = 0.0, 0
    for p, q in zip(c["pts"], r["res"]):
        if not isinstance(q, list): bad += 1; continue
        ref = t.transform(*p)
        d = [abs(a - b) for a, b in zip(ref[:2], q[:2])]
        if geo(c["dst"]): d = [x * 111320 for x in d]   # degrees -> metres
        e = math.hypot(*d)
        if len(p) == 3: e = max(e, abs(ref[2] - q[2]))
        worst = max(worst, e)
    worst_all = max(worst_all, worst)
    op = t.description[:50] if hasattr(t, "description") else ""
    print(f"{c['src']:<18}{c['dst']:<18}{len(c['pts']):>5}{worst:>14.6f}  {op}" + (f"  FAILED {bad}" if bad else ""))
print("worst:", worst_all)
