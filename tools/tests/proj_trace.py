#!/usr/bin/env python3
"""one move's first projectile, frame by frame, on both engines side by side.

usage: tools/tests/proj_trace.py <fighter> <move> [--frames=N] [--every=K] [--fresh] [--thrower]

plays the move the way proj_parity does (same reset, same inputs), then prints, per source frame
from the projectile's first one: where it is relative to its thrower and how far it moved that
frame, in the source's pixels, on SSF2 and here, and what animation it is in here. a path that
starts or ends at another frame, slows at another rate or turns another way shows up as a column
that drifts, which a summary (proj_parity) only reports as a different life or reach.

the source's side comes off its own per-frame record; this engine's off a timer in the thrower's
script that writes the first projectile's place every engine frame (two to a source frame). that
timer runs ahead of the projectiles' own timers and of the frame's movement, so each reading is
how the engine frame before it ended, and a source frame is read two ticks after the projectile
first shows.

`--thrower` prints the thrower's own path instead, from where it stood as the projectile showed: a
projectile that rides its thrower's frame (it sits still, the thrower moves) drifts when the
thrower moves on another frame, and that column says which of the two is off."""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import proj_parity as P  # noqa: E402
import trajectory as T  # noqa: E402
import verify as V  # noqa: E402

# one short eval: the watcher keeps its record on the thrower's exports, read in slices after
WATCH = ("e@p0 (function(){var e=self.exports;e.__pt=[];"
         "e.__ptId=self.addTimer(1,-1,function(){if(e.__pt.length>=400)return;var p=[];"
         "for(q in match.getProjectiles())if(q.exports.__np!=true&&q.getOwner()==self)p.push(q);"
         "var t=Math.round(self.getX())+','+Math.round(self.getY())+','+self.getAnimation();"
         "if(p.length==0){e.__pt.push('-,-,'+t+',-');return;}var q=p[p.length-1];"
         "e.__pt.push(Math.round(q.getX())+','+Math.round(q.getY())+','+t+','+q.getAnimation());"
         "},{persistent:true});return 1;})()")


# the source's record counts projectiles and not the enemies a fighter makes; an enemy here rides
# a projectile, marked so (`__np`, as an item is too), and is left out the same way
assert len(WATCH) <= P.EVAL_LIMIT, f"WATCH is {len(WATCH)} characters, over the engine's {P.EVAL_LIMIT}"


def fm_path(cid, move, frames):
    f = V.Engine(f"projtrace-{cid}")
    f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
    T.entrance_over(f, ("ANIM:INTRO",))
    P.reset(f, False)
    m = f.tell(WATCH)
    f.reply(m, "E:", 10)
    P.arm(f, move)
    f.tell("record")
    P.play(f, move, False)
    f.tick(frames * 2 + 20)
    rows = []
    for i in range(0, frames * 2 + 20, 40):
        got = f.eval(f"match.getCharacters()[0].exports.__pt.slice({i},{i + 40}).join(' ')") or ""
        rows += got.split()
    f.tell("e@p0 self.removeTimer(self.exports.__ptId)")
    f.close(final=True)
    # each row: the projectile's place and animation (None before it shows), and the thrower's
    # place and animation, in the source's units (this engine's are the source's times the size scale)
    k = P.SIZE_SCALE
    out = []
    for r in rows:
        x, y, tx, ty, ta, a = r.split(",", 5)
        thrower = (float(tx) / k, float(ty) / k, ta)
        out.append((None if x == "-" else (float(x) / k, float(y) / k, a), thrower))
    # the watcher ticks with the thrower's timers, ahead of the projectiles' own and of the
    # frame's movement: each reading is how the engine frame before it ended, and the one in the
    # frame a projectile is made sees it before its own first timers place it. so a source frame
    # is read two ticks after the projectile first shows, and every two after that (from the
    # move's start, for `--thrower`, the same way: two ticks after the thrower enters it)
    return out


def ssf2_path(cid, move, frames):
    s = V.Ssf2Engine(f"projtrace-ssf2-{cid}")
    s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2])
    s.faster(8)
    s.tick(200)
    P.reset(s, True)
    P.arm(s, move)
    s.tell("record")
    P.play(s, move, True)
    s.tick(frames + 10)
    mk = s.tell("trace raw")
    s.reply(mk, "TRACERAW:", 20)
    s.tick(2)
    out = []
    for line in s.log()[mk:].splitlines():
        m = re.match(r"(?:<< )?\s*\d+\t(.*)", line)
        if not m:
            continue
        f = m.group(1).split("|")
        if len(f) < 14:
            continue
        # the projectile count and the first one's place sit at 11-13 (a probe's values may follow)
        try:
            n = int(f[11])
            thrower = (float(f[2]), float(f[3]), f[0])
            out.append((None if n == 0 or f[12] == "-" else (float(f[12]), float(f[13]), ""), thrower))
        except ValueError:
            continue
    s.close(final=True)
    return out


def first(rows, test):
    return next((k for k, r in enumerate(rows) if test(r)), None)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) < 2:
        print(__doc__)
        return 2
    cid, move = args[0], args[1]
    frames = int(P.opt("frames", "120"))
    every = int(P.opt("every", "1"))
    if "--fresh" in sys.argv:
        P.fresh(cid)
    fm_rows = fm_path(cid, move, frames)
    src_rows = ssf2_path(cid, move, frames)
    if "--thrower" in sys.argv:
        # from the move's start on each engine: the first frame the thrower shows another animation
        # than it stood in as the recording opened. this engine's reading in the frame the move is
        # entered is after the move's first frame script and before the frame's movement, which is
        # what the source's record of the frame it enters a move in holds (it enters after it
        # moves), so here a source frame is that reading and every second one after it
        start = lambda rows: first(rows, lambda r: r[1][2] != rows[0][1][2]) if rows else None
        i, j = start(src_rows), start(fm_rows)
        src = [] if i is None else src_rows[i:]
        fm = [] if j is None else fm_rows[j::2]
        print(f"{cid} {move}: the thrower, per source frame from the move's start, in source pixels from "
              f"where it stood then (d = how far it moved that frame); p = its first projectile's height "
              f"over it, once out")
        print(f"{'f':>4} | {'ssf2 x':>8} {'y':>7} {'d':>6} {'p':>7} | {'here x':>8} {'y':>7} {'d':>6} {'p':>7}")
        for k in range(0, min(frames, max(len(fm), len(src)))):
            row = []
            for path in (src, fm):
                if k >= len(path):
                    row.append(f"{'':8} {'':7} {'':6} {'':7}")
                    continue
                proj, t = path[k]
                t0, tp = path[0][1], path[k - 1][1] if k else path[k][1]
                d = ((t[0] - tp[0]) ** 2 + (t[1] - tp[1]) ** 2) ** 0.5
                pr = f"{proj[1] - t[1]:7.1f}" if proj else f"{'-':>7}"
                row.append(f"{t[0] - t0[0]:8.1f} {t[1] - t0[1]:7.1f} {d:6.2f} {pr}")
            if k % every == 0:
                print(f"{k:4} | {row[0]} | {row[1]}")
        return 0
    i = first(src_rows, lambda r: r[0] is not None)
    j = first(fm_rows, lambda r: r[0] is not None)
    src = [] if i is None else src_rows[i:]
    fm = [] if j is None else fm_rows[j + 2::2]
    print(f"{cid} {move}: the first projectile, per source frame from its first, in source pixels "
          f"(x, y from its thrower; d = how far it moved that frame, in the world)")
    print(f"{'f':>4} | {'ssf2 x':>8} {'y':>7} {'d':>6} | {'here x':>8} {'y':>7} {'d':>6}  anim")
    prev_s = prev_f = None
    dist = lambda a, b: None if a is None or b is None else ((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2) ** 0.5
    for k in range(0, max(len(fm), len(src))):
        a = src[k][0] if k < len(src) else None
        b = fm[k][0] if k < len(fm) else None
        ds, df = dist(a, prev_s), dist(b, prev_f)
        prev_s, prev_f = a, b
        if a is None and b is None and all(r[0] is None for r in src[k:] + fm[k:]):
            break
        if k % every:
            continue
        ta, tb = src[k][1] if k < len(src) else None, fm[k][1] if k < len(fm) else None
        sa = f"{a[0] - ta[0]:8.1f} {a[1] - ta[1]:7.1f} {ds or 0:6.2f}" if a else f"{'gone':>8} {'':7} {'':6}"
        fb = f"{b[0] - tb[0]:8.1f} {b[1] - tb[1]:7.1f} {df or 0:6.2f}  {b[2]}" if b else f"{'gone':>8}"
        print(f"{k:4} | {sa} | {fb}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
