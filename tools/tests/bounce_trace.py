#!/usr/bin/env python3
"""one move's hit on a standing victim, frame by frame, on both engines side by side.

usage: tools/tests/bounce_trace.py <fighter> <move> [--frames=N] [--at=X] [--dmg=0,40,80]

lands the move the way hit_parity does (the same reset, the same distance, the victim still on
the floor, at 0% or at each damage `--dmg` names), with the hit watch dumping every sample, then
prints the victim's place and speed per source frame from the frame its damage rises. a spike that bounces off the floor shows
where it turns and how fast it leaves: a launch that leaves the floor at another speed, or turns
a frame later, is a column that drifts. heights and speeds are in the source's pixels, measured
from where the victim stood."""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import hit_parity as H  # noqa: E402
import trajectory as T  # noqa: E402
import verify as V  # noqa: E402

ROW = re.compile(r"^\s*(?:<< )?\s*(\d+)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(\S+)\s+(\S+)")


def opt(name, default):
    for a in sys.argv[1:]:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def samples(eng, per, inp, at, frames, dmg=0):
    """(damage, y) per sample of the victim through the move, from the same start hit_parity uses"""
    eng.tell("release all")
    if isinstance(eng, V.Ssf2Engine):
        eng.tell_batch(["e p0.X = 0", f"e p1.X = {at:.2f}", "reset", "e p0.faceRight()"])
    else:
        eng.tell_batch([f"e match.getCharacters()[0].setX(0); match.getCharacters()[1].setX({at:.2f})",
                        "reset", "e match.getCharacters()[0].faceRight()"])
    V.standing(eng, per)
    H.victim_down(eng, per)
    eng.tell(f"dmg p1 {dmg}")
    eng.tick(2 * per)
    if inp[0].startswith("throw"):
        # a throw from a hold, driven as hit_parity drives it (the source has no grab button: a
        # grab is shield with attack there)
        ssf2 = isinstance(eng, V.Ssf2Engine)
        steps = V.fm_steps(inp[1].replace("grab:", "shield+attack:") if ssf2 else inp[1], per, source=True)
        if ssf2:
            eng.tell(f"seq {steps}")
            mark = eng.tell(f"hits p1 {frames * per} raw")
        else:
            mark = eng.tell(f"hits p1 {frames * per} {steps} raw")
    elif isinstance(eng, V.Ssf2Engine):
        eng.tell(f"move {inp[0]} p0")
        mark = eng.tell(f"hits p1 {frames * per} raw")
    else:
        steps = " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" if ":" in t else f"{t}:{per}"
                         for t in inp[1].split())
        mark = eng.tell(f"hits p1 {frames * per} raw {steps}")
    if eng.reply(mark, "HITS:p1", 40, failfast=False) is None:
        return []
    out = []
    for line in eng.log()[mark:].splitlines():
        m = ROW.match(line)
        if m:
            out.append((float(m.group(2)), float(m.group(4)), float(m.group(3))))
    return out


def from_hit(rows, per):
    """one row a source frame, from the sample whose damage first rose, heights from the floor"""
    i = next((k for k in range(1, len(rows)) if rows[k][0] > rows[k - 1][0] + 1e-6), None)
    if i is None:
        return []
    floor, x0 = rows[i - 1][1], rows[i - 1][2]
    picked = rows[i::per]
    return [(d, y - floor, x - x0) for d, y, x in picked]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 2:
        print(__doc__)
        return 2
    cid, move = args[0], args[1]
    frames = int(opt("frames", "60"))
    inp = (move, dict(H.MOVES).get(move, move))
    at = opt("at", None)
    if at is None:
        try:
            at = json.load(open(os.path.join(H.ROOT, "build", "verify", f"hits-{cid}.json")))["moves"][move]["stood"]
        except (OSError, KeyError):
            at = H.AT[0]
    at = float(at)
    dmgs = [float(d) for d in opt("dmg", "0").split(",")]
    s = V.Ssf2Engine(f"bounce-ssf2-{cid}")
    s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2])
    s.wait(("ANIM:",), 120)
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    src = {d: from_hit(samples(s, 1, inp, at, frames, d), 1) for d in dmgs}
    s.close(final=True)
    f = V.Engine(f"bounce-fm-{cid}")
    f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    k = V.SIZE_SCALE
    fm = {d: [(v, y / k, x / k) for v, y, x in from_hit(samples(f, 2, inp, at * k, frames, d), 2)] for d in dmgs}
    f.close(final=True)
    for d in dmgs:
        a_rows, b_rows = src[d], fm[d]
        print(f"{cid} {move} at {at}, victim at {d:g}%: from the frame its damage rose, per source frame "
              f"(x and y from where it stood, dx and dy that frame, source pixels; down is +)")
        print(f"{'f':>4} | {'ssf2 dmg':>8} {'x':>8} {'y':>8} {'dx':>7} {'dy':>7} | {'here dmg':>8} {'x':>8} {'y':>8} {'dx':>7} {'dy':>7}")
        for i in range(max(len(a_rows), len(b_rows))):
            a = a_rows[i] if i < len(a_rows) else None
            b = b_rows[i] if i < len(b_rows) else None
            da = a[1] - a_rows[i - 1][1] if a and i else 0.0
            db = b[1] - b_rows[i - 1][1] if b and i else 0.0
            xa = a[2] - a_rows[i - 1][2] if a and i else 0.0
            xb = b[2] - b_rows[i - 1][2] if b and i else 0.0
            sa = f"{a[0]:8.1f} {a[2]:8.2f} {a[1]:8.2f} {xa:7.2f} {da:7.2f}" if a else f"{'':8} {'':8} {'':8} {'':7} {'':7}"
            sb = f"{b[0]:8.1f} {b[2]:8.2f} {b[1]:8.2f} {xb:7.2f} {db:7.2f}" if b else ""
            print(f"{i:4} | {sa} | {sb}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
