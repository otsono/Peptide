#!/usr/bin/env python3
"""Is the stage actually ANIMATING? Asked per object, not per pixel.

A whole-frame pixel diff cannot answer this: parallax, the fighters and the HUD move
whatever the backdrop does, so a frozen torch hides inside a frame that "changed". This
takes two `tree` dumps and compares each object's OWN animation frame, which is the only
thing that distinguishes an element that is playing from one parked on its last frame.

Take at least THREE dumps, spaced a few seconds apart: two can land on the same frame of a
short loop by chance and report an animating object as frozen.

Usage: for i in 1 2 3 4; do peptide tell tree; sleep 4; done; tools/anim-advanced.py [out.log]
Exits non-zero when an object with a multi-frame animation never advanced in any sample.
"""
import os, re, sys

log = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser('~/.peptide/session/out.log')
lines = open(log).read().splitlines()
marks = [i for i, l in enumerate(lines) if 'TREE:' in l]
if len(marks) < 2:
    sys.exit('need two tree dumps in the log')

def parse(seg):
    out = {}
    for l in seg:
        m = re.search(r'(\w[\w.]*) @\(([-\d.]+),([-\d.]+)\).*?frame=(\d+)/(\d+)', l)
        if m:
            out[(m.group(1), m.group(2), m.group(3))] = (int(m.group(4)), int(m.group(5)))
    return out

# THREE dumps minimum, and an object counts as frozen only if it never moved in ANY of them.
# Two samples is not enough: a 28-frame loop read twice can land on the same frame by chance,
# which reads as frozen and is not. Nine objects "parked" that way turned out to be animating.
if len(marks) < 3:
    sys.exit('need at least three tree dumps in the log (two can alias a short loop)')
# the most recent four: earlier dumps can predate a relaunch, and an object that is simply
# gone by now would drop every later object out of the intersection.
marks = marks[-4:]
segs = [lines[marks[i]:(marks[i + 1] if i + 1 < len(marks) else len(lines))] for i in range(len(marks))]
ds = [parse(s) for s in segs]
common = set.intersection(*[set(d) for d in ds])
multi = [k for k in common if ds[0][k][1] > 1]
stuck = [k for k in multi if len({d[k][0] for d in ds}) == 1]
print(f'{len(common)} objects matched across {len(ds)} samples, {len(multi)} with multi-frame animations')
print(f'  advanced: {len(multi) - len(stuck)}')
print(f'  never advanced: {len(stuck)}')
for k in stuck[:8]:
    print(f'    {k[0]} @({k[1]},{k[2]}) parked at {ds[0][k][0]}/{ds[0][k][1]}')
sys.exit(1 if stuck else 0)
