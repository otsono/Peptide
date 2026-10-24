#!/usr/bin/env python3
"""output_snapshot.py -- prove a converter refactor changed nothing it did not mean to.

Converts the roster (build/verify/ssf2-data.json) in parallel and records a hash of every emitted
text file (scripts, entities, manifests, stats, metas). `--save NAME` keeps the hashes under
build/snapshots/NAME.json; `--against NAME` converts again and lists every file that differs,
appeared or went away, with a unified diff of the first few when `--show` is given (the emitted
text of the new run is kept for that under build/snapshots/NAME.now/).

usage: tools/tests/output_snapshot.py --save before
       tools/tests/output_snapshot.py --against before [--show N] [--only a,b]
"""
import argparse, concurrent.futures as cf, difflib, hashlib, json, os, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.environ.get("PEPTIDE_BIN") or os.path.join(ROOT, "build", "release", "peptide")
SNAP = os.path.join(ROOT, "build", "snapshots")
TEXT = (".hx", ".entity", ".json", ".meta", ".fraytools", ".palettes")

def convert(cid, ssf, work, keep_dir):
    d = os.path.join(work, cid)
    shutil.rmtree(d, ignore_errors=True)
    r = subprocess.run([BIN, "convert", ssf, "-n", cid, "-o", d, "-y"], capture_output=True, text=True)
    out = {}
    if r.returncode != 0:
        return cid, {"__convert_failed__": r.stderr[-300:]}
    for root, _, files in os.walk(d):
        for f in files:
            if not f.endswith(TEXT) or f == "conversion_log.json":
                continue
            p = os.path.join(root, f)
            rel = os.path.relpath(p, d)
            data = open(p, "rb").read()
            out[rel] = hashlib.sha256(data).hexdigest()
            if keep_dir:
                kp = os.path.join(keep_dir, cid, rel)
                os.makedirs(os.path.dirname(kp), exist_ok=True)
                open(kp, "wb").write(data)
    shutil.rmtree(d, ignore_errors=True)
    return cid, out

def run(table, jobs, keep_dir):
    work = os.path.join(ROOT, "build", "snapshots", "work")
    os.makedirs(work, exist_ok=True)
    res = {}
    with cf.ThreadPoolExecutor(jobs) as ex:
        for cid, out in ex.map(lambda kv: convert(kv[0], kv[1], work, keep_dir), sorted(table.items())):
            res[cid] = out
    return res

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--save"); ap.add_argument("--against")
    ap.add_argument("--show", type=int, default=0)
    ap.add_argument("--only", default="")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) - 2))
    a = ap.parse_args()
    table = json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json")))["character"]
    if a.only:
        table = {k: v for k, v in table.items() if k in a.only.split(",")}
    os.makedirs(SNAP, exist_ok=True)
    if a.save:
        keep = os.path.join(SNAP, a.save + ".text")
        shutil.rmtree(keep, ignore_errors=True)
        res = run(table, a.jobs, keep)
        json.dump(res, open(os.path.join(SNAP, a.save + ".json"), "w"), indent=0, sort_keys=True)
        print(f"saved {sum(len(v) for v in res.values())} files across {len(res)} packages as '{a.save}'")
        return 0
    if not a.against:
        ap.print_help(); return 2
    base = json.load(open(os.path.join(SNAP, a.against + ".json")))
    now_dir = os.path.join(SNAP, a.against + ".now")
    shutil.rmtree(now_dir, ignore_errors=True)
    now = run(table, a.jobs, now_dir)
    changed, added, removed = [], [], []
    for cid, files in sorted(now.items()):
        was = base.get(cid, {})
        for rel, h in files.items():
            if rel not in was: added.append(f"{cid}/{rel}")
            elif was[rel] != h: changed.append(f"{cid}/{rel}")
        for rel in was:
            if rel not in files: removed.append(f"{cid}/{rel}")
    print(f"{len(changed)} changed, {len(added)} added, {len(removed)} removed")
    for label, lst in (("changed", changed), ("added", added), ("removed", removed)):
        for x in lst[:40]:
            print(f"  {label}: {x}")
    base_text = os.path.join(SNAP, a.against + ".text")
    for x in changed[: a.show]:
        old = open(os.path.join(base_text, x), encoding="utf-8", errors="replace").read().splitlines()
        new = open(os.path.join(now_dir, x), encoding="utf-8", errors="replace").read().splitlines()
        print("\n".join(list(difflib.unified_diff(old, new, f"before/{x}", f"now/{x}", n=1, lineterm=""))[:60]))
    return 1 if (changed or added or removed) else 0

if __name__ == "__main__":
    sys.exit(main())
