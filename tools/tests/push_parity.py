#!/usr/bin/env python3
"""push_parity.py -- how far apart two standing fighters end up, set down at a distance, on both engines.

both engines push two fighters that stand too close apart, each by its own rule, and a move is
played at the distance they settle at, not the one they were put at (isaac's gaia misses here
because his two fighters stand 42 apart where the source's stand 36). this sets the fighters down
the way `hit_parity.py` does (the same placing and settling) at each distance, waits on the
engines' own frames for the push to finish, and prints where each fighter ended up, in the
source's pixels, from where it was put.

usage: tools/tests/push_parity.py fighter [fighter ...] [--at=10,20,30,40,50]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import hit_parity as H  # noqa: E402
import trajectory as T  # noqa: E402
import verify as V  # noqa: E402


def places(eng, per):
    """(p0 x, p1 x) in the source's pixels"""
    if isinstance(eng, V.Ssf2Engine):
        return V.ssf2_read(eng, "e p0.X"), V.ssf2_read(eng, "e p1.X")
    a = float(eng.eval("match.getCharacters()[0].getX()"))
    b = float(eng.eval("match.getCharacters()[1].getX()"))
    return a / V.SIZE_SCALE, b / V.SIZE_SCALE


def settle(eng, per, at):
    """set down as hit_parity does, then let the push run out"""
    eng.tell("release all")
    if isinstance(eng, V.Ssf2Engine):
        eng.tell_batch(["e p0.X = 0", f"e p1.X = {at:.2f}", "reset", "e p0.faceRight()"])
    else:
        eng.tell_batch([f"e match.getCharacters()[0].setX(0); match.getCharacters()[1].setX({at * V.SIZE_SCALE:.2f})",
                        "reset", "e match.getCharacters()[0].faceRight()"])
    V.standing(eng, per)
    H.victim_down(eng, per)
    first = places(eng, per)
    eng.tick(60 * per)
    return first, places(eng, per)


def run(cid, ats):
    out = {}
    for kind in ("ssf2", "fm"):
        if kind == "ssf2":
            e = V.Ssf2Engine(f"push-ssf2-{cid}")
            e.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2])
            e.wait(("ANIM:",), 120)
            T.entrance_over(e, ("ANIM:ENTRANCE",))
            e.faster(8)
            per = 1
        else:
            e = V.Engine(f"push-fm-{cid}")
            e.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
            T.entrance_over(e, ("ANIM:INTRO",))
            e.faster(8)
            per = 2
        out[kind] = {at: settle(e, per, at) for at in ats}
        e.close(final=True)
    print(f"{cid}: set down `at` apart; where each stands once settled, from where it was put "
          f"(p0 at 0, p1 at `at`), and the gap, in source pixels")
    print(f"{'at':>5} | {'ssf2 p0':>8} {'p1':>7} {'gap':>6} | {'here p0':>8} {'p1':>7} {'gap':>6}")
    for at in ats:
        row = []
        for kind in ("ssf2", "fm"):
            (_, _), (a, b) = out[kind][at]
            row.append(f"{a:8.1f} {b - at:7.1f} {abs(b - a):6.1f}")
        print(f"{at:5} | {row[0]} | {row[1]}")


def main():
    cids = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not cids:
        print(__doc__)
        return 2
    ats = [10.0, 20.0, 30.0, 40.0, 50.0]
    for a in sys.argv[1:]:
        if a.startswith("--at="):
            ats = [float(x) for x in a.split("=", 1)[1].split(",")]
    for cid in cids:
        run(cid, ats)
    return 0


if __name__ == "__main__":
    sys.exit(main())
