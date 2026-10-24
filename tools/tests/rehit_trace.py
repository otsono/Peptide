#!/usr/bin/env python3
"""rehit_trace.py -- where both fighters are, frame by frame, around a move's hits, on both engines.

`hit_parity.py` says a move hits twice here and once on the source; it cannot say why. a second
hit lands when the other fighter is still in reach, so what decides it is where each fighter went
after the first: this plays the move the way `hit_parity.py` does (same distance, same hop) and
prints each engine's own per-frame record (`trace raw`: animation, frame, place, speed, damage,
for every fighter it records), in source frames and source pixels, so the two can be read side by
side. every wait is on the engines' own frames.

this engine's record is taken before the engine moves the fighter in its frame, so its rows show
where the fighter was as the frame began: a move whose own speed starts on its first frame reads a
row late here (mario's down air moves its full speed from its first frame on both engines, which a
timer reading after the move shows).

usage: tools/tests/rehit_trace.py fighter move [distance] [hop] [--victim] [--proj] [--before=N]   (hop: short, full
       or drop; --victim prints the hit fighter's record, damage, place and speed, instead of the
       attacker's; --proj prints the attacker's newest projectile's x on both engines from its first
       frame, through the hits, where a projectile that hits again and again shows where it slows)
"""
import os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
# `--victim`: the hit fighter's own record instead (its damage, place and speed each sample, off the
# engines' hit watch), set before hit_parity is read since it reads it once
VICTIM = "--victim" in sys.argv
if VICTIM:
    os.environ["HP_RAW"] = "1"
import verify as V
import trajectory as T
import hit_parity as H


def raw_rows(eng):
    """each recorded frame of the engine's own record, as its raw row"""
    text = eng.read_trace(eng.tell("trace raw")) or ""
    return [l.split("<<", 1)[-1].strip() for l in text.splitlines() if "|" in l]


PROJ = "--proj" in sys.argv


def proj_path(eng):
    """this engine's watcher record (proj_trace's): the newest projectile's x, one per engine frame"""
    rows = []
    for i in range(0, 400, 40):
        got = eng.eval(f"match.getCharacters()[0].exports.__pt.slice({i},{i + 40}).join(' ')") or ""
        if not got:
            break
        rows += got.split()
    eng.tell("e@p0 self.removeTimer(self.exports.__ptId)")
    return [None if r.startswith("-,") else float(r.split(",")[0]) / V.SIZE_SCALE for r in rows]


# `--before=N`: the move played N times first, untraced, so what it keeps between uses (a stored
# charge: naruto's rasengan charges on one use and lets go on the next) is where the measured run had it
BEFORE = next((int(a.split("=", 1)[1]) for a in sys.argv[1:] if a.startswith("--before=")), 0)


def play(eng, per, move, inp, at, hop):
    for _ in range(BEFORE):
        H.landed(eng, per, (move, inp), at, hop)
    mark = len(eng.log())
    if PROJ and not isinstance(eng, V.Ssf2Engine):
        import proj_trace as PT
        eng.before_record = lambda: eng.reply(eng.tell(PT.WATCH), "E:", 10)
    hit = H.landed(eng, per, (move, inp), at, hop)
    if PROJ:
        if isinstance(eng, V.Ssf2Engine):
            rows = raw_rows(eng)
            return hit, [None if r.split("|")[-2] == "-" else float(r.split("|")[-2]) for r in rows]
        return hit, proj_path(eng)
    if VICTIM:
        # the watch's rows: sample, damage, x, y, moved, vx, vy, state
        rows = [l.split("<<", 1)[-1].strip() for l in eng.log()[mark:].splitlines()
                if re.match(r"\s*(<<\s+)?\s*\d+\s+-?[\d.]+\s+-?[\d.]+\s+-?[\d.]+", l)]
        return hit, rows
    return hit, raw_rows(eng)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if len(args) < 2:
        print(__doc__)
        return 2
    cid, move = args[0], args[1]
    at = float(args[2]) if len(args) > 2 else 34.0
    hop = args[3] if len(args) > 3 else "short"
    inp = dict(H.MOVES).get(move, move)
    s = V.Ssf2Engine(f"rehit-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    src_hit, src_rows = play(s, 1, move, inp, at, hop)
    s.close(final=True)
    f = V.Engine(f"rehit-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    port_hit, port_rows = play(f, 2, move, inp, at * V.SIZE_SCALE, hop)
    f.close(final=True)
    print(f"source hits {src_hit[6] if src_hit else None}")
    print(f"port   hits {port_hit[6] if port_hit else None}")
    if PROJ:
        # from each one's first frame: the source's a row a frame, this engine's every second
        # reading from two after it shows (proj_trace's reading)
        i = next((k for k, v in enumerate(src_rows) if v is not None), None)
        j = next((k for k, v in enumerate(port_rows) if v is not None), None)
        a = [] if i is None else src_rows[i:]
        b = [] if j is None else port_rows[j + 2::2]
        x0, y0 = (a[0] if a else 0), (b[0] if b else 0)
        print("the projectile's x from where it first stood, source pixels, per source frame (dx that frame)")
        print(f"{'f':>4} | {'ssf2 x':>8} {'dx':>6} | {'here x':>8} {'dx':>6}")
        for k in range(max(len(a), len(b))):
            u = a[k] if k < len(a) else None
            v = b[k] if k < len(b) else None
            if u is None and v is None and k > 0:
                break
            pu = a[k - 1] if 0 < k <= len(a) else None
            pv = b[k - 1] if 0 < k <= len(b) else None
            su = f"{u - x0:8.1f} {(u - pu) if u is not None and pu is not None else 0:6.1f}" if u is not None else f"{'-':>8} {'':6}"
            sv = f"{v - y0:8.1f} {(v - pv) if v is not None and pv is not None else 0:6.1f}" if v is not None else f"{'-':>8} {'':6}"
            print(f"{k:4} | {su} | {sv}")
        return 0
    print("source record:")
    for r in src_rows:
        print("  " + r)
    print("port record (two rows a source frame):")
    for r in port_rows:
        print("  " + r)
    return 0


if __name__ == "__main__":
    sys.exit(main())
