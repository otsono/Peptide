#!/usr/bin/env python3
"""box_entity.py -- the boxes the converter extracted, held against the ones it wrote.

`box_runtime.py` holds the extraction against what the source places as it runs; this is the
step after: the entity the conversion emits, against the extraction it was made from. it converts
the fighter with the extraction dumped (`PEPTIDE_DUMP_BOXES`), reads every collision box layer of
every animation of the emitted entity, and on each source frame the extraction has boxes for,
compares the two sets (a source frame is the engine's two, the first of them read):

  missing   an animation that has boxes in the extraction and none at all in the entity (an
            animation cut from a move once emitted its attack with no box: pikachu's skull bash)
  frame     a frame whose boxes the entity lacks or has more of
  place     a box on both whose corner or size sits off by more than the tolerance

an item box turns with the part holding it and is written about its middle, so it is left out.
the entity holds boxes in the source's pixels, as the extraction does. an animation the converter
re-places on purpose (`tumble`, `hurt_heavy`: centred) and a throw it draws turned are left out.
no engine runs: this reads files only.

usage: tools/tests/box_entity.py fighter [fighter ...]   (--all: every installed fighter)
"""
import glob, json, os, re, subprocess, sys, tempfile
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(ROOT, "build", "release", "peptide")
TOL = 0.6
# re-placed on purpose by the conversion
SKIP = {"tumble", "hurt_heavy"}
# an item box turns with the part that holds it and is written about its middle: its own check
SKIP_LAYERS = ("itembox", "cambox")
# and a hold point (`touchBox`, written as a point) and the camera's box are no collision boxes
SKIP_NAMES = ("itemBox", "touchBox", "camBox")


# the engine frames each animation of the last entity read runs for
lengths = {}


def entity_boxes(path):
    """{animation: {engine frame: [(x, y, w, h), ...]}} of every collision box layer"""
    e = json.load(open(path))
    layers = {l["$id"]: l for l in e["layers"]}
    kfs = {k["$id"]: k for k in e["keyframes"]}
    syms = {s["$id"]: s for s in e["symbols"]}
    out = {}
    for a in e["animations"]:
        frames = {}
        for lid in a["layers"]:
            l = layers.get(lid)
            if l is None or l.get("type") != "COLLISION_BOX" or l["name"].startswith(SKIP_LAYERS):
                continue
            f = 0
            for kid in l["keyframes"]:
                k = kfs[kid]
                n = k.get("length", 1)
                s = syms.get(k.get("symbol"))
                if s is not None:
                    box = (s.get("x", 0.0), s.get("y", 0.0), s.get("scaleX", 0.0), s.get("scaleY", 0.0))
                    for g in range(f, f + n):
                        frames.setdefault(g, []).append(box)
                f += n
        out[a["name"]] = frames
        # how long the animation is, off its longest layer
        lengths[a["name"]] = max((sum(kfs[k].get("length", 1) for k in layers[lid]["keyframes"])
                                  for lid in a["layers"] if lid in layers), default=0)
    return out


def turned(cid):
    try:
        return set(json.load(open(os.path.join(ROOT, "characters", cid, "conversion_stats.json"))).get("turned") or [])
    except OSError:
        return set()


def match(src, emitted):
    """(source boxes with no counterpart, emitted boxes with no counterpart, worst offset)"""
    left = list(emitted)
    worst = 0.0
    lost = 0
    for b in src:
        best, bi = None, -1
        for i, c in enumerate(left):
            d = max(abs(b[0] - c[0]), abs(b[1] - c[1]), abs(b[2] - c[2]), abs(b[3] - c[3]))
            if best is None or d < best:
                best, bi = d, i
        if best is None or best > TOL:
            lost += 1
            continue
        worst = max(worst, best)
        left.pop(bi)
    return lost, len(left), worst


def one(cid):
    with tempfile.TemporaryDirectory() as tmp:
        dump = os.path.join(tmp, "boxes.json")
        env = dict(os.environ, PEPTIDE_DUMP_BOXES=dump)
        code = subprocess.run([BIN, "convert", V.source_ssf(cid, "character"), "-n", cid, "-o", tmp, "-y"],
                              env=env, capture_output=True).returncode
        ents = glob.glob(os.path.join(tmp, cid, "library", "entities", "*.entity"))
        if code != 0 or not os.path.exists(dump) or not ents:
            print(f"{cid}: the conversion did not run ({code})")
            return 1
        ext = json.load(open(dump))["animations"]
        # the fighter's own entity: the one with the most animations the extraction names
        names = {a["fm"] for a in ext}
        best = max(ents, key=lambda p: len(names & set(entity_boxes(p))))
        emitted = entity_boxes(best)
        # each animation's start in its clip, as the conversion's own table states it
        starts = {}
        for script in glob.glob(os.path.join(tmp, cid, "library", "scripts", "*", "Script.hx")):
            for m in re.finditer(r'\{ anim: "([^"]+)", label: "([^"]*)", sym: \d+, from: (\d+)', open(script, errors="replace").read()):
                starts.setdefault((m.group(1), m.group(2)), int(m.group(3)))
    skip = SKIP | turned(cid)
    problems = []
    notes = []
    checked = 0
    for a in ext:
        name = a["fm"]
        if name in skip or name not in emitted:
            continue
        src_frames = {int(f): [tuple(b[1:5]) for b in boxes if not b[0].startswith(SKIP_NAMES)]
                      for f, boxes in a["frames"].items()}
        src_frames = {f: b for f, b in src_frames.items() if b}
        if not src_frames:
            continue
        checked += 1
        if not emitted[name]:
            problems.append(f"{name}: missing, {len(src_frames)} source frame(s) of boxes and none written")
            continue
        # an animation cut from later in its clip (an item smash's swing, after its charge) starts
        # on a later source frame: the first one whose boxes its first frame shows
        # where the conversion says it starts, in the clip (`from` in its animation table), else
        # the first source frame whose boxes its first frame shows
        start = 0
        if (name, a.get("label")) in starts:
            start = max(0, starts[(name, a.get("label"))] - 1 - int(a.get("offset") or 0))
        else:
            first = emitted[name].get(0)
            if first and match(src_frames.get(0, []), first)[:2] != (0, 0):
                start = next((f for f, src in sorted(src_frames.items()) if match(src, first)[:2] == (0, 0)), 0)
        bad = []
        for f, src in sorted(src_frames.items()):
            g = f - start
            # a frame before the cut, or past where the animation was cut (at the frame its code
            # ends it), is not written
            if g < 0 or 2 * g >= lengths.get(name, 0):
                continue
            lost, extra, worst = match(src, emitted[name].get(2 * g, []))
            if lost or extra:
                bad.append(f"{f}(-{lost}+{extra})")
        if bad:
            problems.append(f"{name}: frame {', '.join(bad[:8])}" + (" ..." if len(bad) > 8 else "")
                            + (f" (from source frame {start})" if start else ""))
        elif start:
            notes.append(f"{name}: written from source frame {start}")
    print(f"{cid}: {checked} animation(s) with boxes, {len(problems)} differing")
    for p in problems:
        print(f"  {p}")
    for n in notes:
        print(f"  (note) {n}")
    return 1 if problems else 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if "--all" in sys.argv:
        args = sorted(json.load(open(os.path.join(ROOT, "build", "verify", "ssf2-data.json")))["character"])
    if not args:
        print(__doc__)
        return 2
    worst = 0
    for cid in args:
        worst = max(worst, one(cid))
    return worst


if __name__ == "__main__":
    sys.exit(main())
