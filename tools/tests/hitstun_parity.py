#!/usr/bin/env python3
"""hitstun_parity.py -- how long a hit keeps the fighter it lands on in its hurt state, on both engines.

a hit's launch is compared by `hit_parity.py`; what it leaves the fighter in after is not. the
source holds a hit fighter longer the harder it launches it, and the runtime puts that rule in over
this engine's own (`fighters.hx`); a weak launch held too short drops the fighter out of a multi-hit
the source carries it through. this plays each move as `hit_parity.py` does, at the distance the
last sweep landed it (`build/verify/hits-<fighter>.json`), with the watch reporting every sample, and
reads off each engine's record:

  stun    source frames from the last hit until the fighter leaves its hurt, tumble or flying state
          (the watch's end, marked `+`, when it never did)
  launch  the hit fighter's fastest move in the samples after the last hit, in source pixels a
          frame
  path    how far it has gone across and up, in source pixels, 10 source frames after its first
          move faster than a pixel and a half a frame (this engine slides a frozen fighter a little)

every wait is on the engines' own frames.

  --flight  the whole flight: across and up at 5, 10, 20 and 30 source frames, the highest it rose,
            and how far the port's fighter has gone against the source's (HP_DMG sets the damage
            the victim takes it at; a launch at 0% barely leaves the floor)
usage: tools/tests/hitstun_parity.py fighter [move ...] [--all] [--flight]   (default: the grounded moves)
"""
import json, os, re, sys
os.environ["HP_RAW"] = "1"
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T
import hit_parity as H

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MOVES = ["jab1", "tilt_forward", "tilt_up", "tilt_down"]
# the source frames into a flight --flight reads it at
FLIGHT = (5, 10, 20, 30)
ROW = re.compile(r"^\s*(?:<<\s+)?\s*(\d+)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(-?[\d.]+)\s+(\S+)\s+(\S+)\s+(\S+)\s*$")


def rows_of(eng, mark):
    out = []
    for l in eng.log()[mark:].splitlines():
        m = ROW.match(l)
        if m:
            out.append((int(m.group(1)), float(m.group(2)), float(m.group(3)), float(m.group(4)), m.group(8)))
    return out


# a move between two samples further than any launch carries a fighter: it was knocked out and put
# back on the stage inside the watch (lloyd's back throw at 80% read a launch of 569 a frame)
RESPAWN = 100.0


def cut_at_respawn(rows, per, scale):
    """the rows up to a respawn after the first hit, and whether there was one (the watch starts
    as the fighter is put in place)"""
    hit = next((i for i in range(1, len(rows)) if rows[i][1] > rows[i - 1][1] + 1e-6), None)
    if hit is None:
        return rows, False
    for i in range(hit + 1, len(rows)):
        dx, dy = rows[i][2] - rows[i - 1][2], rows[i][3] - rows[i - 1][3]
        if (dx * dx + dy * dy) ** 0.5 * per / scale > RESPAWN:
            return rows[:i], True
    return rows, False


def stun(rows, per, scale):
    """(source frames stunned after the last hit, launch speed a source frame, watch ran out,
    (across, up) 10 source frames into the flight)"""
    hits = [i for i in range(1, len(rows)) if rows[i][1] > rows[i - 1][1] + 1e-6]
    if not hits:
        return None
    last = hits[-1]
    # a hard launch sends the source's fighter flying rather than hurt: the same hold. its hold
    # over in the air, the source's falls ("falling") and this engine's tumbles: either is free to
    # act again, so neither counts (mario's forward throw at 80% read 62 here, the tumble to the
    # floor, against the source's 24)
    hurt = lambda s: any(k in s.lower() for k in ("hurt", "flying"))
    # the state turns a sample after the hit lands on the source: counted from where it turns
    start = next((i for i in range(last, min(len(rows), last + 6)) if hurt(rows[i][4])), last)
    end = next((i for i in range(start, len(rows)) if not hurt(rows[i][4])), None)
    # the launch: the fastest single move in the samples after the hit (its hitstop holds it
    # nearly still first), as the engines' own hit report reads it
    speed = 0.0
    for i in range(last + 1, min(len(rows), last + 1 + 20 * per)):
        dx, dy = rows[i][2] - rows[i - 1][2], rows[i][3] - rows[i - 1][3]
        speed = max(speed, (dx * dx + dy * dy) ** 0.5 * per / scale)
    frames = ((end if end is not None else len(rows)) - last) / per
    # from the first sample faster than a source pixel and a half a frame: this engine slides the
    # fighter a little through the hit's freeze, which is not the flight
    step = lambda i: ((rows[i][2] - rows[i - 1][2]) ** 2 + (rows[i][3] - rows[i - 1][3]) ** 2) ** 0.5 * per / scale
    moved = next((i for i in range(last + 1, len(rows)) if step(i) > 1.5), None)
    path = None
    if moved is not None and moved - 1 + 10 * per < len(rows):
        a, b = rows[moved - 1], rows[moved - 1 + 10 * per]
        path = (abs(b[2] - a[2]) / scale, (a[3] - b[3]) / scale)
    # the whole flight, for --flight: (across, up) at each of FLIGHT's source frames the watch
    # reaches, and the highest it rose, all from where the flight starts
    flight = {}
    peak = None
    if moved is not None:
        a = rows[moved - 1]
        for n in FLIGHT:
            if moved - 1 + n * per < len(rows):
                b = rows[moved - 1 + n * per]
                flight[n] = (abs(b[2] - a[2]) / scale, (a[3] - b[3]) / scale)
        stop = end if end is not None else len(rows)
        peak = max(((a[3] - rows[i][3]) / scale for i in range(moved - 1, stop)), default=None)
    return frames, speed, end is None, path, flight, peak


def play(eng, per, move, at, scale):
    mark = len(eng.log())
    H.landed(eng, per, (move, dict(H.MOVES).get(move, move)), at * scale)
    rows, ko = cut_at_respawn(rows_of(eng, mark), per, scale)
    r = stun(rows, per, scale)
    return None if r is None else r + (ko,)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if not args:
        print(__doc__)
        return 2
    flight = "--flight" in sys.argv
    every = [m for m, _ in H.MOVES] if "--all" in sys.argv else MOVES
    cid, moves = args[0], args[1:] or every
    try:
        reach = {m: r["stood"] for m, r in json.load(open(os.path.join(ROOT, "build", "verify", f"hits-{cid}.json")))["moves"].items()}
    except OSError:
        reach = {}
    s = V.Ssf2Engine(f"stun-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    # a move the last sweep did not land is found the way the sweep finds it: the source tries its
    # distances in order, and this engine plays it where the source landed it first
    src = {}
    for m in moves:
        for at in ([reach[m]] if m in reach else H.AT):
            src[m] = play(s, 1, m, at, 1.0)
            if src[m] is not None:
                reach[m] = at
                break
    s.close(final=True)
    f = V.Engine(f"stun-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    port, port_at = {}, {}
    for m in moves:
        for at in [reach.get(m, 34.0)] + [a for a in H.AT if a != reach.get(m, 34.0)]:
            port[m] = play(f, 2, m, at, V.SIZE_SCALE)
            if port[m] is not None:
                port_at[m] = at
                break
    f.close(final=True)
    print(f"{cid}: stun in source frames, launch in source pixels a frame")
    for m in moves:
        a, b = src[m], port[m]
        fmt = lambda r: "nothing" if r is None else f"stun {r[0]:.0f}{'+' if r[2] else ''} launch {r[1]:.2f}" + ("" if r[3] is None else f" path {r[3][0]:.0f},{r[3][1]:.0f}") + (" KO" if r[6] else "")
        apart = "" if b is None or port_at.get(m) == reach.get(m) else f" (landed here at {port_at[m]:.0f}, the source at {reach.get(m, 34.0):.0f})"
        print(f"  {m:14} source {fmt(a):40} here {fmt(b)}{apart}")
        if flight and a is not None and b is not None:
            pts = lambda r: " ".join(f"{n}:{r[4][n][0]:.0f},{r[4][n][1]:.0f}" for n in FLIGHT if n in r[4])
            print(f"  {'':14}   flight source {pts(a)} peak {a[5] or 0:.0f}")
            print(f"  {'':14}   flight here   {pts(b)} peak {b[5] or 0:.0f}")
            # how far the port's fighter has gone against the source's, at the furthest point both
            # watches reach (the launch's size, whatever its angle)
            far = [n for n in FLIGHT if n in a[4] and n in b[4]]
            if far:
                da, db = a[4][far[-1]], b[4][far[-1]]
                la, lb = (da[0] ** 2 + da[1] ** 2) ** 0.5, (db[0] ** 2 + db[1] ** 2) ** 0.5
                if la > 1:
                    print(f"  {'':14}   distance at {far[-1]}: {lb / la:.2f} of the source's")
    return 0


if __name__ == "__main__":
    sys.exit(main())
