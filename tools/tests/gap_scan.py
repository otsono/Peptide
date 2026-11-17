#!/usr/bin/env python3
"""what the converter leaves out of each installed package, and every package it cannot write.

usage: tools/tests/gap_scan.py [stage|character] [--jobs=N] [--only=a,b] [--out=FILE]

converts each package the installed game has (the table verify.py keeps) into a scratch
directory, reads the port report the conversion writes (`port_report.json`: the gaps of the
fighter or stage, and of each object and projectile), and deletes the output before the next, so
the scan never holds more than a few packages on disk. prints each statement left out ("not
ported: ...") with how many packages leave it out and which, most first, then the other gaps
the same way, then every package that failed to convert; writes the whole tally as json
(`build/verify/gap_scan-<kind>.json` by default). offline: no engine is started, so it runs
beside a live sweep.
"""
import collections
import concurrent.futures as cf
import glob
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import verify as V  # noqa: E402


def opt(name, default):
    for a in sys.argv[1:]:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def convert(kind, cid, src, scratch):
    out = os.path.join(scratch, cid)
    if kind == "stage":
        cmd = [V.BIN, "ssf2", "stage", src, "--out", out]
    else:
        cmd = [V.BIN, "convert", src, "--output", out, "-y"]
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    gaps = []
    reports = glob.glob(os.path.join(out, "**", "port_report.json"), recursive=True)
    for f in reports:
        try:
            d = json.load(open(f, encoding="utf8"))
        except (OSError, ValueError):
            continue
        gaps += d.get("gaps") or []
        for group in ("objects", "projectiles"):
            g = d.get(group)
            for v in (g.values() if isinstance(g, dict) else []):
                if isinstance(v, dict):
                    gaps += v.get("gaps") or []
        for v in (d.get("projectile_gaps") or {}).values():
            gaps += v or []
        for name, why in (d.get("items_without_own_code") or {}).items():
            gaps.append(f"an item whose own code is not written ({name}): {why}")
    shutil.rmtree(out, ignore_errors=True)
    text = p.stdout + p.stderr
    fail = None
    if p.returncode != 0 or not reports:
        fail = (p.stderr.strip().splitlines() or ["?"])[-1]
    for line in text.splitlines():
        if "no fighter script:" in line:
            fail = line.strip()
    return cid, sorted(set(gaps)), fail


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    kind = args[0] if args else "stage"
    V.source_ssf("battlefield", "stage")  # fills the table when it is missing
    table = json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json"), encoding="utf8"))[kind]
    only = [x for x in opt("only", "").split(",") if x]
    subjects = sorted((c, s) for c, s in table.items() if not only or c in only)
    # a package that holds several fighters converts once for all of them
    if kind == "character":
        seen, uniq = set(), []
        for c, s in subjects:
            if s not in seen:
                seen.add(s)
                uniq.append((c, s))
        subjects = uniq
    jobs = int(opt("jobs", "4"))
    scratch = tempfile.mkdtemp(prefix="gap_scan-")
    left_out = collections.defaultdict(list)
    other = collections.defaultdict(list)
    failed = {}
    try:
        with cf.ThreadPoolExecutor(jobs) as ex:
            futs = [ex.submit(convert, kind, c, s, scratch) for c, s in subjects]
            for n, f in enumerate(cf.as_completed(futs), 1):
                cid, gaps, fail = f.result()
                for g in gaps:
                    (left_out if g.startswith("not ported:") else other)[g].append(cid)
                if fail:
                    failed[cid] = fail
                print(f"[{n}/{len(subjects)}] {cid}: {sum(g.startswith('not ported:') for g in gaps)} left out, {len(gaps)} gaps"
                      + (f", FAILED: {fail}" if fail else ""), file=sys.stderr)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    def show(title, table):
        rows = sorted(table.items(), key=lambda kv: (-len(kv[1]), kv[0]))
        print(f"\n== {title}: {len(rows)}")
        for name, who in rows:
            print(f"{len(who):3}  {name}  ({', '.join(sorted(who)[:8])}{', ...' if len(who) > 8 else ''})")
        return rows
    lo = show("statements left out", left_out)
    ot = show("other gaps", other)
    print(f"\n{len(subjects)} {kind}s; {len({c for w in left_out.values() for c in w})} leave a statement out; {len(failed)} failed to convert")
    for cid, why in sorted(failed.items()):
        print(f"  failed: {cid}: {why}")
    out = opt("out", os.path.join(ROOT, "build", "verify", f"gap_scan-{kind}.json"))
    json.dump({"left_out": {k: sorted(v) for k, v in lo}, "gaps": {k: sorted(v) for k, v in ot}, "failed": failed},
              open(out, "w", encoding="utf8"), indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
