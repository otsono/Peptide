#!/usr/bin/env python3
"""box_runtime.py -- the boxes the converter extracted, held against the ones the source places.

The converter reads every hurt, hit and item box off the source's clips (sprite_parser). This
plays moves on SSF2 and, frame after frame, reads the object walk: the clip a fighter is showing,
its frame, and each named box inside it with its world rectangle. The extraction for that clip
and frame (dumped by the converter under PEPTIDE_DUMP_BOXES) is laid over those boxes with the
one translation that fits them best, in either facing, and what is left over is the error:

  missing   a box the source shows that the extraction lacks on that frame (or the reverse)
  size      a box scaled apart from its frame's other boxes, left out of the place fit
  rotated   a box the source turns (reported, not failed: a box here is an upright rectangle)
  place     a box whose corner sits off the rest after the fit (scale and translation are free:
            the walk's corners are in the camera's space)

A clip frame the extraction has no entry for is reported, not guessed. `--offline` re-judges the samples the
last live run saved. Every wait is on the
engine's own frames.

usage: tools/tests/box_runtime.py fighter [input ...] [--reps=N]   (default inputs: the C5 move set)
"""
import json, os, re, subprocess, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(ROOT, "build", "release", "peptide")
OUT = os.path.join(ROOT, "build", "verify", "boxes")
INPUTS = ["attack", "right+attack", "up+attack", "down+attack", "special", "right+special",
          "up+special", "down+special", "jump:3 attack", "jump:3 up+attack", "jump:3 down+attack"]
TOL = 2.5
LINE = re.compile(r"\[object (\S+)\] \"([^\"]*)\" @\((-?[\d.]+),(-?[\d.]+)\)(?: (\d+)x(\d+))?( HIDDEN)?"
                  r"(?: frame=(-?\d+)/(-?\d+))?(?: '([^']*)')?(?: abs=\((-?[\d.]+),(-?[\d.]+)\))?")


def extraction(cid):
    ssf = json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json")))["character"][cid]
    os.makedirs(OUT, exist_ok=True)
    dump = os.path.join(OUT, f"{cid}.extracted.json")
    env = dict(os.environ, PEPTIDE_DUMP_BOXES=dump)
    subprocess.run([BIN, "convert", ssf, "-n", cid, "-o", os.path.join(OUT, "convert"), "-y"],
                   env=env, capture_output=True, check=False)
    return json.load(open(dump))


def boxes_at(ext, cls, frame):
    """the extraction's boxes for a clip class on its (1-based) frame, or None when the clip or
    frame is not one the extraction covers"""
    label = ext["classes"].get(cls.split("::")[-1])
    if label is None:
        return None
    f0 = frame - 1
    best = None
    for a in ext["animations"]:
        if a["label"] == label and a["offset"] <= f0 < a["offset"] + max(a["total"], 1):
            best = a
    if best is None:
        return None
    rel = f0 - best["offset"]
    # a box holds from the frame it is keyed on until the next key
    keys = sorted(int(k) for k in best["frames"])
    at = [k for k in keys if k <= rel]
    if not at:
        return {}
    return {b[0]: b[1:] for b in best["frames"][str(at[-1])]}


def samples(text, ext):
    """(clip class, frame, {box name: (x, y, w, h)}, names of boxes listed with no size) per
    fighter clip shown in one walk. a box is one the extraction knows by name, listed directly
    under the clip (the walk indents a child under its parent; a projectile's boxes sit under its
    own clip)"""
    names = ext.setdefault("_names", {b[0] for a in ext["animations"] for f in a["frames"].values() for b in f})
    out, cur, depth = [], None, -1
    for raw in text.splitlines():
        m = LINE.search(raw)
        if m is None:
            continue
        indent = len(raw) - len(raw.lstrip(" <")) if not raw.lstrip().startswith("<<") else len(raw.split("[object")[0])
        cls, name = m.group(1), m.group(2)
        if m.group(8) and ext["classes"].get(cls.split("::")[-1]):
            cur, depth = (cls, int(m.group(8)), {}, set()), indent
            out.append(cur)
        elif cur is not None and indent <= depth:
            cur = None
        elif cur is not None and name in names and m.group(11):
            if m.group(5) is not None:
                cur[2].setdefault(name, (float(m.group(11)), float(m.group(12)), int(m.group(5)), int(m.group(6))))
            else:
                # listed with no size: there, but nothing to lay over
                cur[3].add(name)
    return out


def _line(ps):
    """least squares global = s * local + t over (local, global) pairs: s, t, worst residual"""
    n = len(ps)
    mx = sum(a for a, _ in ps) / n
    my = sum(b for _, b in ps) / n
    vx = sum((a - mx) ** 2 for a, _ in ps)
    if vx < 1e-6:
        return None
    k = sum((a - mx) * (b - my) for a, b in ps) / vx
    t = my - k * mx
    return k, t, [b - (k * a + t) for a, b in ps]


def fit(src, ext):
    """the leftover per box after the one scale and translation that best lay the extraction over
    the source (the walk's corners are in the camera's space, so the scale is free), in either
    facing: a fighter facing left shows a box's right edge where the extraction has its left.
    None when fewer than three boxes pin the fit."""
    names = sorted(n for n in set(src) & set(ext))
    if len(names) < 3:
        return None
    fy = _line([(ext[n][1], src[n][1]) for n in names])
    if fy is None:
        return None
    best = None
    for face in (1, -1):
        loc = [(ext[n][0] if face > 0 else -(ext[n][0] + ext[n][2])) for n in names]
        fx = _line(list(zip(loc, [src[n][0] for n in names])))
        if fx is None:
            continue
        worst = max(abs(v) for v in fx[2])
        if best is None or worst < best[0]:
            best = (worst, fx)
    if best is None:
        return None
    fx = best[1]
    res = {n: (round(fx[2][i], 1), round(fy[2][i], 1)) for i, n in enumerate(names)}
    # one box far off drags the fit onto the others: laid again without it, the rest are judged
    # by the fit they agree on, and it by that fit
    if max(max(abs(v) for v in r) for r in res.values()) > TOL and len(names) > 3:
        for out in names:
            rest = fit({n: v for n, v in src.items() if n != out}, {n: v for n, v in ext.items() if n != out})
            if rest is not None and all(max(abs(v) for v in r) <= TOL for r in rest.values()):
                # the one left out, by the fit the rest agree on
                return {**rest, out: _place_by(rest, src, ext, out)}
    return res


def _place_by(rest, src, ext, n):
    """where one box sits against the fit the other boxes agree on"""
    names = list(rest)
    fy = _line([(ext[m][1], src[m][1]) for m in names])
    best = None
    for face in (1, -1):
        loc = lambda m: ext[m][0] if face > 0 else -(ext[m][0] + ext[m][2])
        fx = _line([(loc(m), src[m][0]) for m in names])
        if fx and (best is None or max(map(abs, fx[2])) < best[0]):
            best = (max(map(abs, fx[2])), fx, loc)
    _, fx, loc = best
    return (round(src[n][0] - (fx[0] * loc(n) + fx[1]), 1), round(src[n][1] - (fy[0] * ext[n][1] + fy[1]), 1))


def rotated(src, ext):
    """boxes the source shows at another width-to-height ratio than the extraction's, by half
    again: a box the source rotates shows its bounds, and a box here is an upright rectangle"""
    out = []
    for n in set(src) & set(ext):
        if ext[n][2] <= 0 or ext[n][3] <= 0:
            continue
        ga = (src[n][2] + 1) / max(src[n][3], 1)
        ea = (ext[n][2] + 1) / max(ext[n][3], 1)
        if max(ga / ea, ea / ga) > 1.5:
            out.append(n)
    return sorted(out)


def size_outliers(src, ext):
    """boxes whose size is off the scale the frame's other boxes agree on by more than 15% (the
    walk gives sizes in the clip's own space, in whole pixels, so a pixel of slack rides along)"""
    rest = [n for n in set(src) & set(ext) if ext[n][2] > 8 and ext[n][3] > 8]
    if len(rest) < 3:
        return []
    rw = sorted(src[n][2] / ext[n][2] for n in rest)[len(rest) // 2]
    rh = sorted(src[n][3] / ext[n][3] for n in rest)[len(rest) // 2]
    return sorted(n for n in rest if abs(src[n][2] / ext[n][2] - rw) > 0.15 * rw + 1 / ext[n][2]
                  or abs(src[n][3] / ext[n][3] - rh) > 0.15 * rh + 1 / ext[n][3])


def judge(report, ext, inp, cls, frame, got, sizeless=()):
    report["frames"] += 1
    report["samples"].append({"input": inp, "cls": cls, "frame": frame, "boxes": got, "sizeless": sorted(sizeless)})
    want = boxes_at(ext, cls, frame)
    if want is None:
        # a clip the extraction has nothing for is a gap only when the source shows boxes on it
        if got or sizeless:
            report["uncovered"].append(f"{cls}@{frame}")
        return
    # a child named like a box that no clip ever carries as one (a label) is not a box
    known = ext.setdefault("_names", {b[0] for a in ext["animations"] for f in a["frames"].values() for b in f})
    got = {n: v for n, v in got.items() if n in known}
    for n in sorted((set(got) | set(sizeless)) ^ set(want)):
        report["missing"].append(f"{inp} {cls}@{frame} {n} {'extraction only' if n in want else 'source only'}")
    turned = set(rotated(got, want))
    for n in sorted(turned):
        report.setdefault("rotated", set()).add(n)
    got = {n: v for n, v in got.items() if n not in turned}
    odd = set(size_outliers(got, want))
    for n in sorted(odd):
        report["size"].append(f"{inp} {cls}@{frame} {n} source {got[n][2]}x{got[n][3]} extracted {want[n][2]:.1f}x{want[n][3]:.1f}")
    for n, (a, b) in (fit({n: v for n, v in got.items() if n not in odd}, want) or {}).items():
        if max(abs(a), abs(b)) > TOL:
            report["place"].append(f"{inp} {cls}@{frame} {n} off ({a}, {b})")


def finish(report, cid):
    report["rotated"] = sorted(report.get("rotated", ()))
    json.dump(report, open(os.path.join(OUT, f"{cid}.json"), "w"), indent=1)
    print(f"{cid}: {report['frames']} clip frames read, {len(report['uncovered'])} uncovered, "
          f"{len(report['missing'])} missing, {len(report['size'])} size, {len(report['place'])} place"
          + (f" (rotated in the source, upright here: {', '.join(report['rotated'])})" if report["rotated"] else ""))
    for k in ("uncovered", "missing", "size", "place"):
        for line in report[k][:8]:
            print(f"  {k}: {line}")
    return 0 if not (report["missing"] or report["size"] or report["place"]) else 1


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if not args:
        print(__doc__)
        return 2
    cid, inputs = args[0], (args[1:] or INPUTS)
    ext = extraction(cid)
    report = {"fighter": cid, "frames": 0, "uncovered": [], "missing": [], "size": [], "place": [], "samples": []}
    if "--offline" in sys.argv:
        for smp in json.load(open(os.path.join(OUT, f"{cid}.json")))["samples"]:
            judge(report, ext, smp["input"], smp["cls"], smp["frame"], smp["boxes"], smp.get("sizeless", ()))
        return finish(report, cid)
    s = V.Ssf2Engine(f"boxes-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    s.faster(1)
    seen = set()
    # the walk is read while the engine runs, so each move is played several times and the
    # clip frames it lands on add up
    reps = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--reps=")), 6)
    for inp in [i for i in inputs for _ in range(reps)]:
        s.tell("reset"); V.standing(s, 1)
        steps = " ".join(t if ":" in t else f"{t}:1" for t in inp.split())
        m = s.tell(f"seq {steps}")
        s.await_seq(m, len(inp.split()))
        for _ in range(25):
            for cls, frame, got, sizeless in samples(s._tree_text(), ext):
                if (cls, frame) in seen:
                    continue
                seen.add((cls, frame))
                judge(report, ext, inp, cls, frame, got, sizeless)
            s.tick(1)
    s.close(final=True)
    return finish(report, cid)

if __name__ == "__main__":
    sys.exit(main())
