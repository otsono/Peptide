#!/usr/bin/env python3
"""hop_trace.py -- a fighter's short hop, frame by frame, on both engines side by side.

an aerial is played from a short hop on both engines (`hit_parity.py`), and the frame the hop has
reached as the aerial is pressed decides how long the aerial has in the air and how many of its
hits land (lloyd's down air: the source starts it at the top of the hop, here still on the way
up). this taps jump on each engine the way `hit_parity.py` does and prints the fighter's height
over the floor and its animation each source frame from the tap, from each engine's own record.

usage: tools/tests/hop_trace.py fighter [fighter ...] [--frames=N] [--full] [--then=down+attack:3]
       (--full: a held jump; --then: that input pressed so many source frames after the tap, on both
       engines in the same sequence, to see on which frame of the hop each starts what it names)
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import trajectory as T  # noqa: E402
import verify as V  # noqa: E402

FULL = "--full" in sys.argv
THEN = next((a.split("=", 1)[1] for a in sys.argv[1:] if a.startswith("--then=")), None)


def rows_of(eng):
    text = eng.read_trace(eng.tell("trace raw")) or ""
    return [l.split("<<", 1)[-1].strip() for l in text.splitlines() if "|" in l]


def hop(eng, per, frames):
    eng.tell("release all")
    V.standing(eng, per)
    eng.tell("speed 1")
    eng.speed = 1
    eng.tell("record")
    after = int(THEN.rsplit(":", 1)[1]) if THEN else 0
    press = THEN.rsplit(":", 1)[0] if THEN else None
    if isinstance(eng, V.Ssf2Engine):
        seq = "jump:8" if FULL else "jump:1"
        if press:
            seq += (f" none:{after - 1}" if after > 1 else "") + f" {press}:1"
    else:
        seq = f"jump:{8 * per}" if FULL else f"jump+shorthop:{per}"
        if press:
            seq += (f" none:{(after - 1) * per}" if after > 1 else "") + f" {press}:{per}"
    eng.tell(f"seq {seq}")
    eng.tick(frames * per)
    out = []
    for r in rows_of(eng):
        m = re.match(r"\s*\d+\t(.*)", r)
        f = (m.group(1) if m else r).split("|")
        try:
            out.append((f[0], float(f[3])))
        except (IndexError, ValueError):
            continue
    if not out:
        return []
    floor = out[0][1]
    # from the frame it leaves the stand it was recorded in
    i = next((k for k, (a, _) in enumerate(out) if a != out[0][0]), 0)
    picked = out[i::per]
    k = V.SIZE_SCALE if per == 2 else 1.0
    return [(a, (floor - y) / k) for a, y in picked]


def run(cid, frames):
    s = V.Ssf2Engine(f"hop-ssf2-{cid}")
    s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2])
    s.wait(("ANIM:",), 120)
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    src = hop(s, 1, frames)
    s.close(final=True)
    f = V.Engine(f"hop-fm-{cid}")
    f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    fm = hop(f, 2, frames)
    f.close(final=True)
    print(f"{cid} {'full hop' if FULL else 'short hop'}: height over the floor, source pixels, each source frame from the "
          f"first one out of the stand")
    print(f"{'f':>4} | {'ssf2':>7} {'anim':<14} | {'here':>7} {'anim':<14}")
    for k in range(max(len(src), len(fm))):
        a = src[k] if k < len(src) else None
        b = fm[k] if k < len(fm) else None
        sa = f"{a[1]:7.1f} {a[0][:14]:<14}" if a else f"{'':7} {'':14}"
        sb = f"{b[1]:7.1f} {b[0][:14]:<14}" if b else ""
        print(f"{k:4} | {sa} | {sb}")


def main():
    cids = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not cids:
        print(__doc__)
        return 2
    frames = 24
    for a in sys.argv[1:]:
        if a.startswith("--frames="):
            frames = int(a.split("=", 1)[1])
    for cid in cids:
        run(cid, frames)
    return 0


if __name__ == "__main__":
    sys.exit(main())
