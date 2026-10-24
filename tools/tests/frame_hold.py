#!/usr/bin/env python3
"""frame_hold.py -- does a thing the source holds on one frame hold on one frame here?

A clip in the source can stop its own timeline: it then shows that frame until something plays
it again. Nothing here holds a frame on its own, so a converted object runs its animation
through. This spawns a character's own objects on both engines, watches each one's timeline
frame over the engine's frames, and reports what each did:

  held      the object's frame never moved off the one it started on
  ran       the frame advanced
  differ    the source held it and this engine ran it (or the reverse)

Every wait is on the engine's own frames.

usage: tools/tests/frame_hold.py fighter [input ...]
       tools/tests/frame_hold.py --all
"""
import json, os, re, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
# what a character fires or summons comes out of its specials, so those are what is played
INPUTS = ["special", "right+special", "up+special", "down+special",
          "jump:3 special", "jump:3 down+special"]
WATCH = 24          # engine frames each object is watched for after it appears
OBJ = re.compile(r"\[object (\S+)\] \"([^\"]*)\"")
FRAME = re.compile(r"frame=(-?\d+)/(-?\d+)")


def walk_frames(eng):
    """{object name: (its class, the frame its own timeline is on, how many it has)} for every
    object with a timeline. The source lists every clip in its walk with the frame it sits on;
    this engine answers about the fighter and what it made, each asked for its own."""
    out = {}
    if isinstance(eng, V.Ssf2Engine):
        # the fighter's own clip, asked directly: the label it is playing and the frame it sits on
        lab = eng.reply(eng.tell("e p0.MC.currentLabel"), "E:", 15)
        fr = eng.reply(eng.tell("e p0.MC.currentFrame"), "E:", 15)
        if lab and fr:
            try:
                out[f"fighter:{lab}"] = ("fighter", int(float(fr)), 0)
            except ValueError:
                pass
        for raw in eng._tree_text().splitlines():
            o, f = OBJ.search(raw), FRAME.search(raw)
            if o is None or f is None:
                continue
            key = f"{o.group(1).split('::')[-1]}:{o.group(2)}"
            out[key] = (o.group(1), int(f.group(1)), int(f.group(2)))
        return out
    got = _ev(eng, "self.getAnimation() + ':' + self.getCurrentFrame() + ':' + self.getTotalFrames()")
    if got and got.count(":") == 2:
        anim, frame, total = got.split(":")
        out[f"fighter:{anim}"] = ("fighter", int(float(frame)), int(float(total)))
    n = _ev(eng, "match.getProjectiles() == null ? 0 : match.getProjectiles().length")
    for i in range(int(float(n or 0))):
        row = _ev(eng, f"var p = match.getProjectiles()[{i}]; p == null ? null : "
                       f"(p.getAnimation() + ':' + p.getCurrentFrame() + ':' + p.getTotalFrames())")
        if row and row.count(":") == 2:
            anim, frame, total = row.split(":")
            out[f"projectile:{anim}"] = ("projectile", int(float(frame)), int(float(total)))
    return out


def _ev(eng, expr):
    """what the fighter's own interpreter answers, or None"""
    return eng.reply(eng.tell(f"e@p0 {expr}"), "E:", 15)


def watch(eng, per, inputs):
    """{object: [the frames its timeline sat on]} over the objects a character's own moves put on
    the stage, per input. a fighter's own clip is watched too: it is the same question asked of
    it, and the answer is kept under the input that drove it."""
    seen = {}
    for inp in inputs:
        eng.tell("reset")
        V.standing(eng, per)
        steps = " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" if ":" in t else f"{t}:{per}"
                         for t in inp.split())
        m = eng.tell(f"seq {steps}")
        eng.await_seq(m, len(inp.split()))
        for _ in range(WATCH):
            for key, (cls, frame, total) in walk_frames(eng).items():
                row = seen.setdefault(f"{inp}|{key}", {"class": cls, "total": total, "frames": []})
                if len(row["frames"]) < WATCH:
                    row["frames"].append(frame)
            eng.tick(1)
    return seen


def verdict(row):
    fr = row["frames"]
    if len(fr) < 3:
        return "too few reads"
    return "held" if len(set(fr)) == 1 else "ran"


def one_fighter(cid, inputs):
    s = V.Ssf2Engine(f"hold-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    s.faster(1)
    src = watch(s, 1, inputs)
    s.close(final=True)
    f = V.Engine(f"hold-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.faster(1)
    port = watch(f, 1, inputs)
    f.close(final=True)
    report = {"fighter": cid, "source": {k: {**v, "verdict": verdict(v)} for k, v in src.items()},
              "port": {k: {**v, "verdict": verdict(v)} for k, v in port.items()}, "differ": []}
    # the two engines name their objects apart, so the comparison is by what each engine's object
    # did, matched on the source name the conversion kept
    for k, v in report["source"].items():
        inp, _, obj = k.partition("|")
        kind, _, name = obj.partition(":")
        name = name.lower()
        if kind == "fighter":
            # the two engines name the same move apart, so the fighter is paired as the fighter
            mate = next((p for p in report["port"] if p.startswith(f"{inp}|fighter:")), None)
        else:
            mate = next((p for p in report["port"] if name and p.startswith(inp + "|") and name in p.lower()), None)
        if mate is None:
            continue
        if v["verdict"] != report["port"][mate]["verdict"] and "too few" not in v["verdict"]:
            report["differ"].append(f"{k}: the source {v['verdict']} it, this engine "
                                    f"{report['port'][mate]['verdict']} it ({mate})")
    os.makedirs(OUT, exist_ok=True)
    json.dump(report, open(os.path.join(OUT, f"hold-{cid}.json"), "w"), indent=1)
    held = [k for k, v in report["source"].items() if v["verdict"] == "held"]
    print(f"{cid}: {len(report['source'])} object(s) on the source ({len(held)} held), "
          f"{len(report['port'])} here, {len(report['differ'])} differ")
    for line in report["differ"][:12]:
        print(f"  {line}")
    return 1 if report["differ"] else 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if "--all" in sys.argv:
        roster = sorted(json.load(open(os.path.join(OUT, "ssf2-data.json")))["character"])
        worst = 0
        for cid in (args or roster):
            worst = max(worst, one_fighter(cid, INPUTS))
        return worst
    if not args:
        print(__doc__)
        return 2
    return one_fighter(args[0], args[1:] or INPUTS)


if __name__ == "__main__":
    sys.exit(main())
