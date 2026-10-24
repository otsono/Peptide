#!/usr/bin/env python3
"""calibrate.py -- re-measure the physics constants the converter scales by, on both engines.

The converter's physics rests on a handful of numbers that were measured, not read: the size
scale (`stats.jsonc` size_multiplier, from which every velocity and acceleration scale follows)
and the integration step a launch speed loses crossing engines. When either engine updates,
these are what move. This drives the same inputs on SSF2 and on the converted fighter in
Fraymakers and reports, per fighter:

  walk   the port's speed over fifteen held frames from a stand, over the source's: the size scale
  jump   a held jump's apex and the source frame it lands on, both engines
  hop    a tapped jump's (short hop's) apex and landing frame, both engines

and fails when the size ratio is off stats.jsonc by more than 6%, an apex by more than 6% (or
3 px), or a landing by more than one source frame. Every wait is on the engines' own frames.

usage: tools/tests/calibrate.py [fighter ...]        (default: mario fox kirby)
       results are also written to build/verify/calibration.json
"""
import json, os, re, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def configured_size():
    raw = open(os.path.join(ROOT, "crates", "ssf2-converter", "mappings", "character", "stats.jsonc"), encoding="utf-8").read()
    m = re.search(r'"size_multiplier":\s*([\d.]+)', raw)
    return float(m.group(1)) if m else 1.3


def arc(rows, per, scale):
    """apex (source px above the start) and the source frame of the landing: the first frame
    after the apex back on the floor it left"""
    if not rows:
        return None, None
    ys = [float(r[3]) for r in rows]
    y0 = ys[0]
    top = min(range(len(ys)), key=lambda i: ys[i])
    apex = (y0 - ys[top]) / scale
    land = next((i for i in range(top, len(ys)) if ys[i] >= y0 - 1), None)
    return round(apex, 1), (None if land is None or apex < 1 else land // per)


def measure(cid, size):
    out = {"fighter": cid}
    s = V.Ssf2Engine(f"calibrate-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        return {"fighter": cid, "error": "the SSF2 session never came up"}
    s.faster(1)
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    # the walk is the moves gate's own measure (fifteen source frames from the middle of the floor)
    ws = V.walk_speed(s, 1)
    src = {"jump": T.play(s, "jump:14", 1, 40), "hop": T.play(s, "jump", 1, 30)}
    s.close(final=True)
    f = V.Engine(f"calibrate-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        return {"fighter": cid, "error": "the Fraymakers session never came up"}
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    wp = V.walk_speed(f, 2)
    port = {"jump": T.play(f, "jump:14", 2, 40), "hop": T.play(f, "jump", 2, 30)}
    f.close(final=True)
    # the port's speed is per engine frame, at twice the source's rate
    out["walk"] = {"source": ws and round(ws, 3), "port": wp and round(wp, 3),
                   "size_ratio": round(wp * 2 / ws, 3) if ws and wp else None}
    for k in ("jump", "hop"):
        a_s, l_s = arc(src[k], 1, 1.0)
        a_p, l_p = arc(port[k], 2, size)
        out[k] = {"source_apex": a_s, "port_apex": a_p, "source_lands": l_s, "port_lands": l_p}
    return out


def verdicts(r, size):
    bad = []
    ratio = (r.get("walk") or {}).get("size_ratio")
    if ratio is None:
        bad.append("walk not measured")
    elif abs(ratio - size) / size > 0.06:
        bad.append(f"walk says size {ratio} against {size}")
    for k in ("jump", "hop"):
        j = r.get(k) or {}
        a_s, a_p = j.get("source_apex"), j.get("port_apex")
        if a_s is None or a_p is None:
            bad.append(f"{k} apex not measured"); continue
        if abs(a_p - a_s) > max(3.0, 0.06 * a_s):
            bad.append(f"{k} apex {a_p} against {a_s}")
        l_s, l_p = j.get("source_lands"), j.get("port_lands")
        if l_s is not None and l_p is not None and abs(l_s - l_p) > 1:
            bad.append(f"{k} lands on {l_p} against {l_s}")
    return bad


def main():
    fighters = [a for a in sys.argv[1:] if not a.startswith("-")] or ["mario", "fox", "kirby"]
    size = configured_size()
    results, failed = [], 0
    for cid in fighters:
        r = measure(cid, size)
        r["problems"] = verdicts(r, size) if "error" not in r else [r["error"]]
        results.append(r)
        failed += bool(r["problems"])
        print(json.dumps(r))
    os.makedirs(os.path.join(ROOT, "build", "verify"), exist_ok=True)
    json.dump({"size_multiplier": size, "results": results}, open(os.path.join(ROOT, "build", "verify", "calibration.json"), "w"), indent=1)
    print(f"{len(fighters) - failed} of {len(fighters)} fighters within calibration")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
