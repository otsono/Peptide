#!/usr/bin/env python3
"""strike_tally.py -- what the converter leaves unported, counted, against a committed baseline.

Every line the port cannot carry is written as a marked comment (`// [no counterpart: x]`,
`// [SSF2-only: x]`, `// [always false here]`, ...). The count of each kind is the converter's
floor: a change that makes a kind appear, or grow, has started striking something it used to
port, and the roster gates only notice if a driven move happens to reach that line.

The roster (build/verify/ssf2-data.json) is converted in parallel into one scratch directory,
each fighter's emitted scripts are also put through tools/check-emitted.py and
tools/tests/unanswerable-calls.py, and the per-kind counts are compared with the baseline.

usage: tools/tests/strike_tally.py [--jobs N] [--out DIR] [--update] [--only a,b]
  exit 1 when a kind is new or grew, or an emitted script has a problem or an unanswered call;
  --update rewrites the baseline (after a change that ported something, or on purpose)
"""
import argparse, collections, concurrent.futures as cf, json, os, re, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BASELINE = os.path.join(ROOT, "tools", "tests", "strike_tally.json")
BIN = os.environ.get("PEPTIDE_BIN") or os.path.join(ROOT, "build", "release", "peptide")
MARK = re.compile(r"// \[(no counterpart: [^\]]+|SSF2-only: [\w.]+|SSF2-dead|always false here|always true here|"
                  r"it has no animation of this name|that is the character's animation, not this one's|"
                  r"[^\]]{0,50}has no [\w.]+)\]")

def tally(dirpath):
    c = collections.Counter()
    for root, _, files in os.walk(dirpath):
        for f in files:
            if not f.endswith((".hx", ".entity")):
                continue
            text = open(os.path.join(root, f), encoding="utf-8", errors="replace").read()
            for m in MARK.finditer(text):
                # one kind per struck NAME, not per argument list: `no counterpart: a, a` is `a`
                k = m.group(1)
                if k.startswith("no counterpart: "):
                    k = "no counterpart: " + k[len("no counterpart: "):].split(",")[0].strip()
                c[k] += 1
    return c

def one(cid, ssf, out):
    d = os.path.join(out, cid)
    shutil.rmtree(d, ignore_errors=True)
    r = subprocess.run([BIN, "convert", ssf, "-n", cid, "-o", d, "-y"], capture_output=True, text=True)
    if r.returncode != 0:
        return cid, None, [f"convert failed: {r.stderr.strip()[-200:]}"]
    problems = []
    ce = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "check-emitted.py"), d], capture_output=True, text=True)
    if ce.returncode != 0 or not ce.stdout.strip().endswith("0 problem(s) across the emitted scripts"):
        problems.append("check-emitted: " + (ce.stdout.strip().splitlines() or ["?"])[-1])
    uc = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "tests", "unanswerable-calls.py"), d, "--fail"], capture_output=True, text=True)
    if uc.returncode != 0:
        problems.append("unanswered calls: " + " | ".join(l.strip() for l in uc.stdout.strip().splitlines()[:5]))
    c = tally(d)
    shutil.rmtree(d, ignore_errors=True)
    return cid, c, problems

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) - 2))
    ap.add_argument("--out", default=os.path.join(ROOT, "build", "tally"))
    ap.add_argument("--update", action="store_true")
    ap.add_argument("--only", default="")
    a = ap.parse_args()
    table = json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json")))["character"]
    if a.only:
        table = {k: v for k, v in table.items() if k in a.only.split(",")}
    os.makedirs(a.out, exist_ok=True)
    kinds, who, problems = collections.Counter(), collections.defaultdict(set), {}
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        for cid, c, probs in ex.map(lambda kv: one(kv[0], kv[1], a.out), sorted(table.items())):
            if probs:
                problems[cid] = probs
            if c is None:
                continue
            kinds.update(c)
            for k in c:
                who[k].add(cid)
            print(f"{cid:14} {sum(c.values()):5} struck" + (f"   PROBLEMS: {'; '.join(probs)}" if probs else ""), flush=True)
    current = {k: {"count": kinds[k], "fighters": sorted(who[k])} for k in sorted(kinds)}
    if a.update or not os.path.exists(BASELINE):
        if a.only:
            print("refusing to write a baseline from a partial roster (--only)"); return 1
        json.dump({"total": sum(kinds.values()), "kinds": current}, open(BASELINE, "w"), indent=1, sort_keys=True)
        print(f"baseline written: {sum(kinds.values())} struck lines, {len(kinds)} kinds")
        return 1 if problems else 0
    base = json.load(open(BASELINE))["kinds"]
    worse, better = [], []
    for k, v in current.items():
        if k not in base:
            worse.append(f"  + NEW {k}: {v['count']}  {v['fighters'][:6]}")
        elif not a.only and v["count"] > base[k]["count"]:
            # a partial roster (--only) can only say a kind is new, not that it grew
            worse.append(f"  + {k}: {base[k]['count']} -> {v['count']}  {v['fighters'][:6]}")
    if not a.only:
        for k, v in base.items():
            if current.get(k, {}).get("count", 0) < v["count"]:
                better.append(f"  - {k}: {v['count']} -> {current.get(k, {}).get('count', 0)}")
    print(f"\n{sum(kinds.values())} struck lines in {len(kinds)} kinds (baseline {sum(v['count'] for v in base.values())})")
    if better:
        print("fewer than the baseline (run with --update to keep):"); print("\n".join(better))
    if worse:
        print("MORE than the baseline:"); print("\n".join(worse))
    if problems:
        print("emitted-script problems:")
        for cid, probs in sorted(problems.items()):
            print(f"  {cid}: {'; '.join(probs)}")
    return 1 if (worse or problems) else 0

if __name__ == "__main__":
    sys.exit(main())
