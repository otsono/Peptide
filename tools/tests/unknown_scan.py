#!/usr/bin/env python3
"""what the converted code still asks for that nothing answers, over every installed package.

usage: tools/tests/unknown_scan.py [stage|character] [--jobs=N] [--only=a,b] [--out=FILE]

converts each package the installed game has (the table verify.py keeps) into a scratch
directory, reads the runtime report the conversion writes (its `unknown` names and `problems`,
the stage's and each object's), and deletes the output before the next, so the scan never holds
more than a few packages on disk. prints each unknown name with how many packages ask for it and
which, most asked first, and writes the whole tally as json (`build/verify/unknown_scan-<kind>.json`
by default). offline: no engine is started, so it runs beside a live sweep.
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
    unknown, problems = set(), []
    reports = glob.glob(os.path.join(out, "**", "runtime_report.json"), recursive=True)
    for f in reports:
        try:
            d = json.load(open(f, encoding="utf8"))
        except (OSError, ValueError):
            continue
        def each(v):
            return list(v.values()) if isinstance(v, dict) else list(v or [])
        parts = [d] + each(d.get("objects")) + each(d.get("projectiles"))
        for part in parts:
            if isinstance(part, dict):
                unknown |= set(part.get("unknown") or [])
                problems += part.get("problems") or []
    shutil.rmtree(out, ignore_errors=True)
    failed = p.returncode != 0 and not reports
    return cid, sorted(unknown), problems, (p.stderr.strip().splitlines() or ["?"])[-1] if failed else None


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
    scratch = tempfile.mkdtemp(prefix="unknown_scan-")
    asked = collections.defaultdict(list)
    problems, failed = {}, {}
    try:
        with cf.ThreadPoolExecutor(jobs) as ex:
            futs = [ex.submit(convert, kind, c, s, scratch) for c, s in subjects]
            for n, f in enumerate(cf.as_completed(futs), 1):
                cid, unknown, probs, fail = f.result()
                for u in unknown:
                    asked[u].append(cid)
                if probs:
                    problems[cid] = probs
                if fail:
                    failed[cid] = fail
                print(f"[{n}/{len(subjects)}] {cid}: {len(unknown)} unknown" + (f", FAILED: {fail}" if fail else ""), file=sys.stderr)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    rows = sorted(asked.items(), key=lambda kv: (-len(kv[1]), kv[0]))
    for name, who in rows:
        print(f"{len(who):3}  {name}  ({', '.join(sorted(who)[:8])}{', ...' if len(who) > 8 else ''})")
    print(f"\n{len(rows)} distinct unknown names over {len({c for w in asked.values() for c in w})} of {len(subjects)} {kind}s; "
          f"{len(problems)} with problems, {len(failed)} failed to convert")
    for cid, why in sorted(failed.items()):
        print(f"  failed: {cid}: {why}")
    out = opt("out", os.path.join(ROOT, "build", "verify", f"unknown_scan-{kind}.json"))
    json.dump({"unknown": {k: sorted(v) for k, v in rows}, "problems": problems, "failed": failed},
              open(out, "w", encoding="utf8"), indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
