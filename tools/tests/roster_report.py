#!/usr/bin/env python3
"""roster_report.py -- what a `hit_parity.py --all` sweep left, in one place, and what changed.

reads each fighter's `build/verify/hits-<fighter>.json` and reports:

  totals      moves both engines landed, the moves that differ in something, the lines that
              differ, the ones listed apart as accepted (`hit_parity.py`'s `~` lines), and the reaches
              listed apart as at an edge (`?` lines: the source lands it where this engine did too)
  by kind     how many difference lines are a reach, a hit count, damage, a launch or an angle
  unlanded    the moves the source lands and this engine does not (gaps here), and how many the
              source lands nothing with (those are not played here)

--save=NAME keeps this sweep's files under `build/verify/roster-NAME/`; --against=NAME reports each
difference line and unlanded move that is new or gone since that saved sweep. offline: nothing is
played.

usage: tools/tests/roster_report.py [--save=NAME] [--against=NAME] [--list]
"""
import collections, glob, json, os, shutil, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VERIFY = os.path.join(ROOT, "build", "verify")


def load(folder):
    out = {}
    for path in sorted(glob.glob(os.path.join(folder, "hits-*.json"))):
        try:
            d = json.load(open(path))
        except (OSError, ValueError):
            continue
        out[d.get("fighter") or os.path.basename(path)[5:-5]] = d
    return out


def kind(line):
    what = line.split(": ", 1)[1] if ": " in line else line
    if "lands it at" in what:
        return "reach"
    if what.startswith("hits "):
        return "hits"
    if "damage on the source" in what:
        return "damage"
    if what.startswith("launch "):
        return "launch"
    if what.startswith("sent at "):
        return "angle"
    return "other"


def lines(sweep):
    diff, unl = set(), set()
    for f, d in sweep.items():
        diff |= {f"{f} {p}" for p in d.get("problems", [])}
        unl |= {f"{f} {u}" for u in d.get("unlanded", [])}
    return diff, unl


def main():
    opts = dict(a[2:].split("=", 1) for a in sys.argv[1:] if a.startswith("--") and "=" in a)
    sweep = load(VERIFY)
    if not sweep:
        print("no hit_parity results in build/verify")
        return 1
    landed = sum(len(d.get("moves", {})) for d in sweep.values())
    diff, unl = lines(sweep)
    moves = {" ".join(x.split(" ")[:2]).rstrip(":") for x in diff}
    accepted = sum(len(d.get("accepted", [])) for d in sweep.values())
    edges = sum(len(d.get("edges", [])) for d in sweep.values())
    print(f"{len(sweep)} fighters, {landed} moves landed on both, {len(moves)} differ "
          f"({len(diff)} lines), {accepted} accepted, {edges} at the edge of their reach")
    by = collections.Counter(kind(x.split(" ", 1)[1]) for x in diff)
    print("by kind: " + ", ".join(f"{k} {n}" for k, n in by.most_common()))
    src_only = sorted(x for x in unl if "port landed nothing" in x)
    print(f"unlanded: {len(src_only)} only the source lands, {len(unl) - len(src_only)} the source lands nothing with "
          f"(not played here)")
    if "--list" in sys.argv:
        for x in sorted(diff):
            print("  " + x)
        for x in src_only:
            print("  " + x)
    if "against" in opts:
        old = load(os.path.join(VERIFY, f"roster-{opts['against']}"))
        if not old:
            print(f"no saved sweep named {opts['against']}")
        else:
            common = set(old) & set(sweep)
            od, ou = lines({f: old[f] for f in common})
            nd, nu = lines({f: sweep[f] for f in common})
            print(f"against {opts['against']} ({len(common)} fighters in both): "
                  f"{len(od)} -> {len(nd)} difference lines, "
                  f"{sum('port landed' in x for x in ou)} -> {sum('port landed' in x for x in nu)} only the source lands")
            for x in sorted(od - nd):
                print("  - " + x)
            for x in sorted(nd - od):
                print("  + " + x)
            for x in sorted((ou - nu)):
                print("  - " + x)
            for x in sorted((nu - ou)):
                print("  + " + x)
    if "save" in opts:
        dest = os.path.join(VERIFY, f"roster-{opts['save']}")
        os.makedirs(dest, exist_ok=True)
        for path in glob.glob(os.path.join(VERIFY, "hits-*.json")):
            shutil.copy(path, dest)
        print(f"saved as {opts['save']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
