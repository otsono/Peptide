#!/usr/bin/env python3
"""a source stage's clips frame by frame.

usage: tools/tests/stage_watch.py <stage> <pattern> [--from=F] [--samples=N] [--every=K]

boots the source stage with a stock pair of fighters, runs it fast to frame F of the match, then reads its display tree every K frames, N times, at the
engine's own rate, and prints for each read the frame and label of every clip whose class or name
matches `pattern` (a regex). a run of reads that do not change is printed once. the port's side
of the same clip is read with `e` in the stage's interpreter (`Planes.get("background").currentFrame`).

for what a stage's clips do between the moments the gates look: a plane its code sends to a label
that plays on from it, a clip a frame script sends back, a phase that comes on a timer.
"""
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V  # noqa: E402


def opt(name, default):
    for a in sys.argv[1:]:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def rows(text, pat):
    out = []
    for line in text.splitlines():
        if not pat.search(line):
            continue
        m = re.search(r"\[object (\S+)\] \"([^\"]*)\".*?frame=(\S+)(?: '([^']*)')?", line)
        if m:
            out.append(f"{m.group(1)} \"{m.group(2)}\" {m.group(3)}{' ' + m.group(4) if m.group(4) else ''}")
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 2:
        print(__doc__)
        return 2
    sid, pat = args[0], re.compile(args[1], re.I)
    start, samples, every = int(opt("from", "0")), int(opt("samples", "60")), int(opt("every", "2"))
    e = V.Ssf2Engine(f"stagewatch-{sid}")
    e.boot(["--char", "mario,mario", "--stage", sid])
    # fast to the moment asked for, then the engine's own rate, so each read is K frames on
    e.faster(8)
    if start:
        e.tick(start)
    e.tell("speed 1")
    last = None
    for i in range(samples):
        mark = len(e.log())
        e.tell("tree 8")
        e.tick(every)
        now = " || ".join(rows(e.log()[mark:], pat)) or "-"
        if now != last:
            print(f"{start + i * every:>6}: {now}")
            last = now
    e.close(final=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
