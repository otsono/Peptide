#!/usr/bin/env python3
"""move_trace.py -- any input, frame by frame, on both engines side by side.

the gates say a move ran a different length; this says where the two parted. it presses the same
input on each engine the way `input_parity.py` does (from the stage middle, the other fighter well
off to the left, both stood still) and prints, each source frame, the animation, its frame and
the fighter's position on each engine, from each one's own record. with `--at` the other fighter
stands that far in front facing back, as the gates stand it. a move that branches on what
the fighter touches (a landing, a hit, a wall) shows the frame each engine took the branch.

usage: tools/tests/move_trace.py fighter "input" [--frames=N] [--at=X] [--from=X]
       the input is the harness's `a:N b` form in source frames (`right:8 right+special`);
       --at puts the other fighter X source pixels in front instead of off to the left;
       --from starts the fighter X source pixels right of where it would (under a platform, say);
       --floor writes the source fighter's height too, as the gates place it;
       --all prints every frame (a still stretch is one line otherwise: a throw, a freeze)
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import trajectory as T  # noqa: E402
import verify as V  # noqa: E402

PASSING = ("stand", "idle")


def opt(name, default):
    return next((a.split("=", 1)[1] for a in sys.argv[1:] if a.startswith(f"--{name}=")), default)


def rows(eng, inp, frames, per, source):
    """(animation, frame, x, y) each engine frame of the record, from the first one out of the stand"""
    eng.tell("release all")
    V.standing(eng, per)
    eng.tell("speed 1")
    eng.speed = 1
    eng.tell("record")
    # the source has no grab button: a grab is shield with attack there
    steps = V.fm_steps(inp.replace("grab", "shield+attack"), per, source=True) if source else V.fm_steps(inp, per)
    eng.tell(f"seq {steps}")
    eng.tick(frames * per)
    out = []
    seen = set()
    for line in (eng.read_trace(eng.tell("trace raw")) or "").splitlines():
        if "|" not in line:
            continue
        body = line.split("<<", 1)[-1].strip()
        # each recorded frame once, by its own index: the text can carry the record twice
        idx = body.split("\t", 1)[0].strip()
        if "\t" in body and idx.isdigit():
            if idx in seen:
                continue
            seen.add(idx)
        f = body.split("\t")[-1].split("|")
        try:
            # the source's row carries its animation's frame further along than this engine's
            out.append((f[0], f[10] if source else f[4], float(f[2]), float(f[3])))
        except (IndexError, ValueError):
            continue
    i = next((k for k, r in enumerate(out) if r[0] not in PASSING), len(out))
    return out[i:]


def run(cid, inp, frames, at, start=0.0):
    s = V.Ssf2Engine(f"mtrace-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print(f"{cid}: the source never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    mid = (V.ssf2_read(s, "e p0.X") + V.ssf2_read(s, "e p1.X")) / 2
    mid += start
    other = mid - 30 + at if at is not None else mid - 200
    s.tell("reset")
    # one in front faces back, as the gates stand it (`verify.py` C6)
    s.tell_batch([f"e p0.X = {mid - 30:.1f}", f"e p1.X = {other:.1f}", "e p0.faceRight()"]
                 + (["e p1.faceLeft()"] if at is not None else []))
    # and at the floor's height, written as the gates write it (`--floor`): the height off its own
    # record once it stands, since the reflection read answers the spawn's
    if "--floor" in sys.argv:
        V.standing(s, 1)
        s.tell("record")
        s.tick(4)
        ends = re.findall(r"end=\(([-\d.]+),([-\d.]+)\)", s.read_trace(s.tell("trace")) or "")
        if ends:
            s.tell(f"e p0.Y = {float(ends[-1][1]):.2f}")
    src = rows(s, inp, frames, 1, True)
    s.close(final=True)

    f = V.Engine(f"mtrace-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print(f"{cid}: this engine never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    c0, c1 = "match.getCharacters()[0]", "match.getCharacters()[1]"
    mid = (float(f.eval(f"{c0}.getX()")) + float(f.eval(f"{c1}.getX()"))) / 2
    k = V.SIZE_SCALE
    mid += start * k
    other = mid + (-30 + at) * k if at is not None else mid - 200 * k
    f.tell("reset")
    f.tell(f"e {c0}.setX({mid - 30 * k:.1f}); {c1}.setX({other:.1f}); {c0}.faceRight()"
           + (f"; {c1}.faceLeft()" if at is not None else ""))
    fm = rows(f, inp, frames, 2, False)[::2]
    f.close(final=True)

    # positions from where each one left the stand, in source pixels
    def rel(rs, scale):
        if not rs:
            return []
        x0, y0 = rs[0][2], rs[0][3]
        return [(a, n, (x - x0) / scale, (y0 - y) / scale) for a, n, x, y in rs]
    src, fm = rel(src, 1.0), rel(fm, k)
    print(f"{cid} `{inp}`: each source frame from the first one out of the stand "
          f"(x forward, y up, source pixels from there; this engine's frame is its own, two a source frame)")
    print(f"{'f':>4} | {'ssf2':<18} {'fr':>4} {'x':>7} {'y':>7} | {'here':<18} {'fr':>4} {'x':>7} {'y':>7}")
    last = None
    for i in range(max(len(src), len(fm))):
        a = src[i] if i < len(src) else None
        b = fm[i] if i < len(fm) else None
        sa = f"{a[0][:18]:<18} {a[1]:>4} {a[2]:7.1f} {a[3]:7.1f}" if a else " " * 38
        sb = f"{b[0][:18]:<18} {b[1]:>4} {b[2]:7.1f} {b[3]:7.1f}" if b else ""
        # a long still run reads as one line
        key = (a[0], round(a[2], 1), round(a[3], 1)) if a else None, (b[0], round(b[2], 1), round(b[3], 1)) if b else None
        if key == last and "--all" not in sys.argv:
            continue
        last = key
        print(f"{i:4} | {sa} | {sb}")
    return 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 2:
        print(__doc__)
        return 2
    at = opt("at", None)
    return run(args[0], args[1], int(opt("frames", "60")), float(at) if at is not None else None,
               float(opt("from", "0")))


if __name__ == "__main__":
    sys.exit(main())
