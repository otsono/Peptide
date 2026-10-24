#!/usr/bin/env python3
"""refresh.py -- convert fighters from the game the engines run, and pack them, in parallel.

A live check runs the PACKED build, so a conversion that is not packed is not what gets played.
Each fighter is converted from the installed game's own data (the same file `verify.py` compares
against), then packed; fighters run side by side, one process each, except those converted from one file,
which share what that conversion leaves behind and go one after another.

usage: tools/refresh.py fighter [fighter ...] [-j N]
       tools/refresh.py --all [-j N]
"""
import concurrent.futures as cf, json, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verify as V


def one(cid):
    t = time.time()
    r = subprocess.run([V.BIN, "convert", V.source_ssf(cid, "character"), "-n", cid,
                        "-o", os.path.join(V.ROOT, "characters"), "-y"], capture_output=True, text=True)
    if r.returncode != 0:
        return cid, f"CONVERT FAILED: {r.stderr.strip()[-300:]}", time.time() - t
    project = os.path.join(V.ROOT, "characters", cid, f"{cid}.fraytools")
    r = subprocess.run([V.BIN, "pack", "--project", project], capture_output=True, text=True)
    if r.returncode != 0:
        return cid, f"PACK FAILED: {r.stderr.strip()[-300:]}", time.time() - t
    return cid, "ok", time.time() - t


def main():
    argv = sys.argv[1:]
    jobs = os.cpu_count() or 4
    args = []
    i = 0
    while i < len(argv):
        a = argv[i]
        if a == "-j" and i + 1 < len(argv) and argv[i + 1].isdigit():
            jobs = int(argv[i + 1])
            i += 1
        elif a.startswith("-j") and a[2:].isdigit():
            jobs = int(a[2:])
        elif not a.startswith("-"):
            args.append(a)
        i += 1
    if "--all" in sys.argv:
        args = sorted(json.load(open(os.path.join(V.ROOT, "build", "verify", "ssf2-data.json")))["character"])
    if not args:
        print(__doc__)
        return 2
    t = time.time()
    bad = 0
    # fighters converted from the same file share what that conversion leaves behind, so they go
    # one after another in one worker; different files go side by side
    groups = {}
    for cid in args:
        groups.setdefault(V.source_ssf(cid, "character"), []).append(cid)
    with cf.ThreadPoolExecutor(max_workers=jobs) as ex:
        for results in ex.map(lambda g: [one(c) for c in g], groups.values()):
            for cid, status, secs in results:
                print(f"{cid:14} {status} ({secs:.0f}s)", flush=True)
                bad += status != "ok"
    print(f"{len(args)} fighter(s) in {time.time() - t:.0f}s, {bad} failed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
