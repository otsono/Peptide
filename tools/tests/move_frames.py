#!/usr/bin/env python3
"""move_frames.py -- what a move LOOKS like, frame by frame, on both engines, side by side.

The gates read animation names, lengths and positions off each engine's own record. None of
them look at what is drawn: a pose off its body, a part left floating, an effect that never
shows, a fighter drawn facing the wrong way all pass every gate. This drives the same input on
each engine, parks the camera on the fighter, and captures the frame at the same points in the
move (counted in SOURCE frames: this engine runs two for each), then writes one sheet per input
with the source's row above this engine's.

usage: tools/tests/move_frames.py fighter [input ...] [--at=0,3,6,10,15,22] [--out=DIR]
       inputs are the harness's (`attack`, `right+attack`, `jump:3 down+attack`, `special`, ...)
"""
import os, re, subprocess, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
INPUTS = ["attack", "right+attack", "up+attack", "down+attack", "special", "right+special",
          "up+special", "down+special", "jump:3 attack", "jump:3 right+attack",
          "jump:3 up+attack", "jump:3 down+attack", "jump:3 left+attack"]


def opt(name, default):
    for a in sys.argv:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def hide(eng, on, who=0):
    """a fighter drawn or not: the driven one hidden for a reference frame of everything else,
    the other one hidden throughout so its own idling is not mistaken for the move"""
    if isinstance(eng, V.Ssf2Engine):
        eng.tell(f"e p{who}.MC.alpha = {0 if on else 1}")
    else:
        eng.tell(f"e match.getCharacters()[{who}].setAlpha({0 if on else 1})")
    eng.tick(2)


def snap(eng, path):
    mk = eng.tell(f"shot {path}")
    eng.reply(mk, "SHOT", 15)
    return path if os.path.exists(path) else None


def shots(eng, per, inp, at, tag, out):
    """play `inp` from a standing start and capture the frame `at[i]` source frames in, plus a
    reference frame with the fighter hidden (the crop is found by what differs from it)"""
    eng.tell("release all")
    eng.tell("reset")
    V.standing(eng, per)
    name = re.sub(r'[^a-z0-9]+', '_', inp)
    hide(eng, True, 1)
    hide(eng, True)
    ref = snap(eng, os.path.join(out, f"{tag}_{name}_ref.png"))
    hide(eng, False)
    steps = " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" if ":" in t else f"{t}:{per}"
                     for t in inp.split())
    m = eng.tell(f"seq {steps}")
    eng.await_seq(m, len(inp.split()))
    got, last = [], 0
    for f in at:
        eng.tick(max(0, (f - last) * per))
        last = f
        got.append(snap(eng, os.path.join(out, f"{tag}_{name}_{f:02d}.png")))
    return ref, got


def fighter_box(ref, frames):
    """one box around everything the fighter touched across the move: the pixels that differ
    from the frame taken with it hidden (the other fighter and the stage cancel out)"""
    from PIL import ImageChops
    box = None
    for im in frames:
        if im is None:
            continue
        diff = ImageChops.difference(im.convert("RGB"), ref.convert("RGB")).convert("L")
        bb = diff.point(lambda v: 255 if v > 40 else 0).getbbox()
        if bb:
            box = bb if box is None else (min(box[0], bb[0]), min(box[1], bb[1]), max(box[2], bb[2]), max(box[3], bb[3]))
    return box


def sheet(rows, out_png):
    """each engine's row cropped to the box its fighter covered, the source above this engine"""
    from PIL import Image
    cell = (240, 240)
    canvas = Image.new("RGB", (cell[0] * max(len(r[1]) for r in rows), cell[1] * len(rows)), (40, 40, 40))
    for j, (ref, frames) in enumerate(rows):
        ref_im = Image.open(ref) if ref else None
        ims = [Image.open(p) if p else None for p in frames]
        box = fighter_box(ref_im, ims) if ref_im else None
        for i, im in enumerate(ims):
            if im is None:
                continue
            im = im.convert("RGB")
            if box:
                # square, with room around it, so every cell of a row is framed the same
                cx, cy = (box[0] + box[2]) // 2, (box[1] + box[3]) // 2
                half = max(box[2] - box[0], box[3] - box[1]) // 2 + 16
                im = im.crop((cx - half, cy - half, cx + half, cy + half))
            canvas.paste(im.resize(cell, Image.NEAREST), (i * cell[0], j * cell[1]))
    canvas.save(out_png)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if not args:
        print(__doc__)
        return 2
    cid, inputs = args[0], (args[1:] or INPUTS)
    at = [int(x) for x in opt("at", "0,3,6,10,15,22").split(",")]
    out = opt("out", os.path.join(ROOT, "build", "verify", "frames", cid))
    os.makedirs(out, exist_ok=True)
    s = V.Ssf2Engine(f"frames-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    s.faster(1)
    src = {inp: shots(s, 1, inp, at, "src", out) for inp in inputs}
    s.close(final=True)
    f = V.Engine(f"frames-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.faster(1)
    port = {inp: shots(f, 2, inp, at, "fm", out) for inp in inputs}
    errs = [l for l in f.log().splitlines() if "SCRIPTERR" in l or "Interpret Error" in l]
    f.close(final=True)
    for inp in inputs:
        png = os.path.join(out, f"sheet_{re.sub(r'[^a-z0-9]+', '_', inp)}.png")
        sheet([src[inp], port[inp]], png)
        print(png)
    if errs:
        print(f"{len(errs)} script error line(s) on this engine, first: {errs[0][:200]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
