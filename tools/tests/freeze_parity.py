#!/usr/bin/env python3
"""freeze_parity.py -- how long a hit freezes the attacker, on both engines.

A landed hit holds both fighters still for a few frames. The converter carries the freeze a move
STATES; a move that states none gets each engine's own default, and the two defaults need not
agree -- a hit that hangs longer or shorter than the source's is the first thing a player feels.
This plays each move twice on each engine, once into empty air and once into the other fighter
standing in reach, and reads how much longer the attacker's own animation lasted when it hit. That
difference is the freeze, in source frames (this engine runs two for each).

usage: tools/tests/freeze_parity.py fighter [move ...]    (moves by this engine's name)
"""
import json, os, re, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T
import hit_parity as H

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
MOVES = ["jab1", "tilt_forward", "tilt_up", "tilt_down"]


def fresh(eng, cid):
    m = eng.tell(f"spawn {cid},{cid} {V.CHAR_STAGE_FM}")
    eng.reply(m, "LAUNCHED", 60, failfast=False)
    t0 = time.time()
    while "ANIM:INTRO" not in eng.log()[m:] and time.time() - t0 < 60:
        eng.next_line(1.0)
    T.entrance_over(eng, ("ANIM:INTRO",))
    eng.faster(1)


def played(eng, per, move, inp, at, cid):
    """frames (in source frames) the attacker spent in the move's own animation, the other fighter
    `at` source pixels in front (far away = a whiff)"""
    ssf2 = isinstance(eng, V.Ssf2Engine)
    if not ssf2:
        fresh(eng, cid)
    eng.tell("release all")
    eng.tell("reset")
    if ssf2:
        eng.tell_batch(["e p0.X = 0", f"e p1.X = {at:.2f}"])
    else:
        eng.tell(f"e match.getCharacters()[0].setX(0); match.getCharacters()[1].setX({at * V.SIZE_SCALE:.2f})")
    V.standing(eng, per)
    if ssf2:
        eng.tell("e p0.faceRight()")
    else:
        eng.tell("e match.getCharacters()[0].faceRight(); match.getCharacters()[1].faceLeft()")
    eng.tick(2 * per)
    eng.tell("record")
    if ssf2:
        eng.tell(f"move {move} p0")
    else:
        steps = " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" if ":" in t else f"{t}:{per}"
                         for t in inp.split())
        eng.tell(f"seq {steps}")
    eng.tick(90 * per)
    runs = V.trace_runs(eng.read_trace(eng.tell("trace")))
    moving = [(a, n) for a, n in runs if not a.lower().startswith(("stand", "idle", "walk", "fall", "land", "hurt", "turn", "jump"))]
    return (moving[0][0], moving[0][1] / per) if moving else (None, 0)


GROUND = ["jab1", "tilt_forward", "tilt_up", "tilt_down", "dash_attack",
          "strong_forward_attack", "strong_up_attack", "strong_down_attack"]


def stated(cid, move):
    """(damage, the attacker freeze the move states in source frames or None) off the conversion"""
    import glob
    for p in glob.glob(os.path.join(ROOT, "characters", cid, "library/scripts/*/HitboxStats.hx")):
        t = open(p, encoding="utf-8", errors="replace").read()
        m = re.search(rf'\n\t{re.escape(move)}:\s*\{{(.*?)\n\t\}}', t, re.S)
        if not m:
            continue
        dmg = re.search(r'damage:\s*([\d.]+)', m.group(1))
        sh = re.search(r'selfHitstop:\s*(-?\d+)', m.group(1))
        self_freeze = None if (sh is None or sh.group(1) == "-1") else int(sh.group(1)) / 2 - 1
        return (float(dmg.group(1)) if dmg else None), self_freeze
    return None, None


def source_only(cids):
    """the source's attacker freeze for every ground move of each fighter, beside its damage"""
    rows = []
    for cid in cids:
        s = V.Ssf2Engine(f"freeze-src-{cid}")
        if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
            continue
        T.entrance_over(s, ("ANIM:ENTRANCE",))
        s.faster(1)
        for m in GROUND:
            dmg, st = stated(cid, m)
            if dmg is None:
                continue
            (a, w), (_, h) = played(s, 1, m, m, 240, cid), played(s, 1, m, m, 22, cid)
            rows.append((cid, m, dmg, st, a, w, h, h - w))
            print(f"  {cid:12} {m:22} dmg {dmg:5.1f} stated {st}  {a}: {w:.0f} -> {h:.0f} = {h - w:+.0f}", flush=True)
        s.close(final=True)
    json.dump(rows, open(os.path.join(OUT, "freeze-source.json"), "w"), indent=1)
    return 0


def still_frames(rows):
    """source frames the victim holds still after the one its damage rises on, before the first it
    moves on (rows: damage, y, x per frame, as bounce_trace samples them). the frame the hit lands
    on is not counted: a fighter already moving still moves on it (lloyd's side special), as this
    engine's does, whose freeze starts after it"""
    i = next((k for k in range(1, len(rows)) if rows[k][0] > rows[k - 1][0] + 1e-6), None)
    if i is None:
        return None
    y0, x0 = rows[i][1], rows[i][2]
    return next((k - i - 1 for k in range(i, len(rows))
                 if abs(rows[k][1] - y0) > 0.01 or abs(rows[k][2] - x0) > 0.01), None)


def victims(cid, moves):
    """how long each move holds the fighter it hits, on both engines: the source's off where the
    victim stands (it holds still from the frame its damage rises), this engine's as it hands the
    victim its freeze at the hit (it shakes a frozen victim in place, so its position cannot say).
    each move lands as hit_parity lands it, at the distance hit_parity found it reaches."""
    import bounce_trace as B
    try:
        reach = json.load(open(os.path.join(OUT, f"hits-{cid}.json")))["moves"]
    except (OSError, KeyError):
        reach = {}
    moves = [m for m in moves if not reach or m in reach]
    at = {m: float(reach.get(m, {}).get("stood") or H.AT[0]) for m in moves}
    inputs = dict(H.MOVES)
    # the engine loads the package, so it is built from `characters/` first (as hit_parity does)
    import glob, subprocess
    project = next(iter(glob.glob(os.path.join(ROOT, "characters", cid, "*.fraytools"))), None)
    if project and "--no-pack" not in sys.argv:
        subprocess.run([os.path.join(ROOT, "build", "release", "peptide"), "pack", "--project", project],
                       capture_output=True, check=False)
    s = V.Ssf2Engine(f"vfreeze-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    src = {m: still_frames(B.samples(s, 1, (m, inputs.get(m, m)), at[m], 24)) for m in moves}
    s.close(final=True)
    f = V.Engine(f"vfreeze-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    f.reply(f.tell("e@p1 \"fz:\" + (function() { self.exports.__fz = -1; self.addEventListener("
                   "GameObjectEvent.HIT_RECEIVED, function(?e) { if (self.exports.__fz < 0) "
                   "self.exports.__fz = self.getHitstop(); }, {persistent: true}); return 1; })()"), "E:fz:", 10)
    port = {}
    for m in moves:
        f.tell("e@p1 self.exports.__fz = -1")
        B.samples(f, 2, (m, inputs.get(m, m)), at[m] * V.SIZE_SCALE, 24)
        got = f.reply(f.tell("e@p1 \"fq:\" + self.exports.__fz"), "E:fq:", 10)
        try:
            port[m] = float(got) / 2 if got is not None and float(got) >= 0 else None
        except ValueError:
            port[m] = None
    f.close(final=True)
    print(f"== {cid}  victim freeze, source frames")
    for m in moves:
        dmg, _ = stated(cid, m)
        print(f"  {m:22} dmg {dmg}  source {src[m]}  port {port[m]}")
    return 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if "--victim" in sys.argv:
        return victims(args[0], args[1:] or GROUND)
    if "--source-only" in sys.argv:
        return source_only(args)
    if not args:
        print(__doc__)
        return 2
    cid, moves = args[0], (args[1:] or MOVES)
    inputs = dict(H.MOVES)
    # the smashes and the rest, by the control that starts each
    inputs.update({"strong_forward_attack": "strong_forward", "strong_up_attack": "strong_up",
                   "strong_down_attack": "strong_down", "special_side": "right+special",
                   "throw_forward": "grab right", "throw_back": "grab left"})
    s = V.Ssf2Engine(f"freeze-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    s.faster(1)
    src = {m: (played(s, 1, m, inputs.get(m, m), 240, cid), played(s, 1, m, inputs.get(m, m), 22, cid)) for m in moves}
    s.close(final=True)
    f = V.Engine(f"freeze-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    port = {m: (played(f, 2, m, inputs.get(m, m), 240, cid), played(f, 2, m, inputs.get(m, m), 22, cid)) for m in moves}
    f.close(final=True)
    print(f"== {cid}  (source frames: whiff -> hit = freeze)")
    rows = {}
    for m in moves:
        (sa, sw), (_, sh) = src[m]
        (pa, pw), (_, ph) = port[m]
        rows[m] = {"source": [sa, sw, sh, sh - sw], "port": [pa, pw, ph, ph - pw]}
        print(f"  {m:16} source {sa}: {sw:.0f} -> {sh:.0f} = {sh - sw:+.0f}    port {pa}: {pw:.1f} -> {ph:.1f} = {ph - pw:+.1f}")
    os.makedirs(OUT, exist_ok=True)
    json.dump(rows, open(os.path.join(OUT, f"freeze-{cid}.json"), "w"), indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
