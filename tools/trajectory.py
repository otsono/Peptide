#!/usr/bin/env python3
"""one move, frame by frame, on both engines: the source's own record beside the port's.

    python3 tools/trajectory.py <character> "<input>" [source frames]

boots SSF2, plays the input from a stand, reads the engine's per-frame record (label, clip
frame, y, y speed); then the same on Fraymakers, whose record carries the animation, its frame
and y (its y speed is the difference between frames). the port runs two engine frames a source
frame, so its rows are paired, and its speeds are shown in the source's units (over the
velocity scale) so the two columns read the same. what this answers: which frame a move's
own y speed writes land on, what a declared stat does to the fall, where a landing happens.
never paced on a wall clock: every wait is the engine's own frame count.
"""
import os, re, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verify as V

VELOCITY_SCALE = V.VELOCITY_SCALE


def entrance_over(eng, tags, seconds=20):
    t0 = time.time()
    while time.time() - t0 < seconds:
        text = eng.log()[eng.mark0:]
        if any(t in text and text.rfind("ANIM:STAND") > text.rfind(t) for t in tags):
            return
        eng.next_line(1.0)


def raw_rows(eng):
    mark_t = eng.tell("trace raw")
    head = eng.reply(mark_t, "TRACERAW:", 20)
    want = int(re.search(r"(\d+) frames", head).group(1)) if head and re.search(r"(\d+) frames", head) else 0
    def have():
        return sum(1 for line in eng.log()[mark_t:].splitlines() if re.match(r"(?:<< )?\s*\d+\t", line))
    t0 = time.time()
    while have() < want and time.time() - t0 < 10:
        if eng.next_line(1.0) is None:
            break
    rows = []
    for line in eng.log()[mark_t:].splitlines():
        m = re.match(r"(?:<< )?\s*(\d+)\t(.*)", line)
        if m:
            rows.append(m.group(2).split("|"))
    return rows


def play(eng, inp, per, frames, recovery=False, at=0.0):
    eng.tell("reset"); V.standing(eng, per)
    if recovery:
        # parked high and falling from rest for a few frames before the input, as the harness does
        # a recovery (V.RECOVERY_LEAD), so the whole rise and fall play out in the air
        if isinstance(eng, V.Ssf2Engine):
            # its standing height off its own record: the reflection read answers the spawn's
            eng.tell("record"); eng.tick(1)
            rows = raw_rows(eng)
            y = float(rows[-1][3])
            eng.tell_batch([f"e p0.X = {at:.2f}", "e p0.faceRight()", f"e p0.Y = {y - 260.0:.2f}"])
        else:
            c = "match.getCharacters()[0]"
            y = float(eng.eval(f"{c}.getY()"))
            eng.tell(f"e {c}.setX({at * V.SIZE_SCALE:.1f}); {c}.setY({y - 260.0 * V.SIZE_SCALE:.1f}); "
                     f"{c}.setYVelocity({c}.getCharacterStat(\"gravity\") * 1.5); {c}.toState(CState.FALL); {c}.faceRight()")
        eng.tick(V.RECOVERY_LEAD * per)
    eng.tell("record")
    # a tapped jump is a short hop on the source; the port names it (verify.drive_inputs)
    tokens = inp.split()
    if not isinstance(eng, V.Ssf2Engine):
        tokens = ["jump+shorthop" if t == "jump" else t for t in tokens]
    steps = " ".join(f"{s.split(':')[0]}:{int(s.split(':')[1]) * per}" if ":" in s else f"{s}:{per}" for s in tokens)
    m = eng.tell(f"seq {steps}")
    eng.await_seq(m, len(inp.split()))
    eng.tick(frames * per)
    return raw_rows(eng)


def main():
    if len(sys.argv) < 3:
        print(__doc__); sys.exit(2)
    park = "--park" in sys.argv or any(a.startswith("--at=") for a in sys.argv)
    # where to park the fighter, in the source's pixels (the harness parks every other input off
    # centre: pass the same spot to see the same move)
    at = next((float(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--at=")), 0.0)
    argv = [a for a in sys.argv[1:] if not a.startswith("--")]
    cid, inp = argv[0], argv[1]
    frames = int(argv[2]) if len(argv) > 2 else 60

    s = V.Ssf2Engine(f"trajectory-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up"); sys.exit(1)
    s.faster(1)
    entrance_over(s, ("ANIM:ENTRANCE",))
    if park:
        # the verify harness's spots: the fighter at 0, the other one OTHER_AT away
        s.tell_batch([f"e p1.X = {at + V.OTHER_AT:.2f}", f"e p0.X = {at:.2f}", "e p0.faceRight()"])
    recovery = "--recovery" in sys.argv
    src = play(s, inp, 1, frames, recovery, at)
    s.close(final=True)

    f = V.Engine(f"trajectory-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up"); sys.exit(1)
    entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    if park:
        y = float(f.eval("match.getCharacters()[0].getY()"))
        f.tell_batch([f"e match.getCharacters()[1].setX({(at + V.OTHER_AT) * V.SIZE_SCALE:.1f}); match.getCharacters()[1].setY({y:.1f})",
                      f"e match.getCharacters()[0].setX({at * V.SIZE_SCALE:.1f}); match.getCharacters()[0].setY({y:.1f}); match.getCharacters()[0].faceRight()"])
    port = play(f, inp, 2, frames, recovery, at)
    f.close(final=True)

    # source rows: label|frame|x|y|xs|ys|...   port rows: anim|state|x|y|frame
    print(f"{'sf':>3}  {'source':<16}{'fr':>4}{'x':>8}{'y':>9}{'ys':>8}   |  {'port':<18}{'fr':>4}{'x':>8}{'y':>9}{'vy/scale':>9}")
    prev_y = None
    for i, r in enumerate(src):
        if len(r) < 6: continue
        lab, fr, x, y, ys = r[0], r[1], float(r[2]), float(r[3]), float(r[5])
        # the frame the pose clip is on (the record's eleventh field), which is what plays a move;
        # the root's frame only names the label
        if len(r) > 10 and r[10] not in ("", "-"): fr = r[10]
        p = port[2 * i + 1] if 2 * i + 1 < len(port) else None
        p0 = port[2 * i] if 2 * i < len(port) else None
        if p is not None and len(p) >= 4:
            py = float(p[3]); px = float(p[2]); pfr = p[4] if len(p) > 4 else ""
            py0 = float(p0[3]) if p0 is not None and len(p0) >= 4 else py
            vy = (py - py0) / VELOCITY_SCALE  # one engine frame's step, in the source's units per source frame... x2 frames
            vy = (py - (float(port[2 * i - 1][3]) if 2 * i - 1 >= 0 and len(port[2 * i - 1]) >= 4 else py0)) / VELOCITY_SCALE
            print(f"{i:>3}  {lab:<16}{fr:>4}{x:>8.1f}{y:>9.2f}{ys:>8.2f}   |  {p[0]:<18}{pfr:>4}{px:>8.1f}{py:>9.2f}{vy:>9.2f}")
        else:
            print(f"{i:>3}  {lab:<16}{fr:>4}{x:>8.1f}{y:>9.2f}{ys:>8.2f}   |  (no port row)")


if __name__ == "__main__":
    main()
