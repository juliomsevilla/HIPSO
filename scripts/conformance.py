#!/usr/bin/env python3
"""Checks hipso-crs against recorded reference values (scripts/conformance_cases.json).

    cargo build -p hipso-crs --release --example batch
    python3 scripts/conformance.py

Exit status 1 when a case is off by more than its tolerance. The horizontal agreement with
PROJ is checked by scripts/check_spain_crs.py and scripts/check_world_crs.py.
"""
import json, os, subprocess, sys, tempfile

here = os.path.dirname(os.path.abspath(__file__))
spec = json.load(open(os.path.join(here, "conformance_cases.json")))
cases = [c for c in spec["cases"] if not c.get("src_ellipsoidal")]
skipped = [c["name"] for c in spec["cases"] if c.get("src_ellipsoidal")]
batch = [{"src": c["src"], "dst": c["dst"], "pts": [c["point"]]} for c in cases]
path = os.path.join(tempfile.mkdtemp(), "cases.json")
json.dump(batch, open(path, "w"))
exe = os.path.join(here, "..", "target", "release", "examples", "batch")
out = json.loads(subprocess.run([exe, path], capture_output=True, text=True, check=True).stdout)
bad = 0
for c, r in zip(cases, out):
    got = r["res"][0]
    if not isinstance(got, list):
        print(f"FAIL {c['name']}: no result"); bad += 1; continue
    err = abs(got[2] - c["z"])
    ok = err <= c.get("tol", spec["tolerance_m"])
    bad += not ok
    print(f"{'ok  ' if ok else 'FAIL'} {c['name']}: {got[2]:.4f} (expected {c['z']}, error {err:.4f})")
for n in skipped:
    print(f"skip {n}: covered by the unit test of crates/crs (ellipsoidal heights on a projected CRS)")
sys.exit(1 if bad else 0)
