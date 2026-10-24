#!/usr/bin/env python3
"""what the source engine's display holds while a move plays, frame-accurately.

usage: tools/tests/ssf2_watch.py <fighter> <move> <pattern> [--samples=N] [--every=K] [--speed=S]

plays the move on SSF2 the way proj_parity does, then walks the display tree every K of the
engine's frames and prints, for each walk, the engine's own frame count and every display object
whose class or name matches `pattern` (a regex), with its frame, label and place. the source runs
slowed (`--speed`, a quarter by default) and each walk is stamped with the engine's clock asked
right before it, so a row's frame is the frame the walk was taken on, not a guess from a tick
count (the engine does not stop while it is asked).

for the things the source's per-frame record does not carry: an enemy a fighter makes (pac-man's
hydrant), an effect, a clip inside a projectile.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import proj_parity as P  # noqa: E402
import verify as V  # noqa: E402


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 3:
        print(__doc__)
        return 2
    cid, move, pattern = args[0], args[1], re.compile(args[2], re.I)
    samples = int(P.opt("samples", "60"))
    every = int(P.opt("every", "3"))
    speed = P.opt("speed", "0.25")
    s = V.Ssf2Engine(f"ssf2watch-{cid}")
    s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2])
    s.faster(8)
    s.tick(200)
    P.reset(s, True)
    P.arm(s, move)
    s.tell(f"speed {speed}")
    start = len(s.log())
    P.play(s, move, True)
    for _ in range(samples):
        s.tell("clock")
        s.tell("tree 8")
        s.tick(every)
    s.tick(2)
    text = s.log()[start:]
    s.close(final=True)
    first = None
    for part in text.split(">> clock")[1:]:
        c = re.search(r"CLOCK:(\d+)", part)
        frame = int(c.group(1)) if c else None
        if frame is not None and first is None:
            first = frame
        rows = []
        for line in part.splitlines():
            m = re.match(r"(?:<< )?\s*\[object (\S+)\] \"([^\"]*)\" @\(([-\d.]+),([-\d.]+)\).*?frame=(\S+)(?: '([^']*)')?", line)
            if m and (pattern.search(m.group(1)) or pattern.search(m.group(2))):
                rows.append(f"{m.group(1)} {m.group(5)} {m.group(6) or ''} @({m.group(3)},{m.group(4)})")
        at = "?" if frame is None or first is None else frame - first
        print(f"{at:>5} | " + (" || ".join(rows) if rows else "-"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
