#!/usr/bin/env python3
"""ssf2_probe.py -- any value of the source's fighter, read every frame while a move plays.

the source's per-frame record carries a fixed set of fields (label, frame, place, speeds, damage,
the pose clip's frame, the first projectile). a rule the port gets wrong is often decided by a
value it leaves out, and a read through `e` lands only every dozen frames or so, too coarse for a
rule that turns on a single frame. this hands the record dotted paths from the fighter (public
properties only, `;` between them) and it reads each one on every frame from then on.

usage: tools/tests/ssf2_probe.py fighter move "path;path..." [distance] [frames] [--victim="path;..."] [--hop=N] [--dmg=N] [--placed]

prints one row per frame of the fighter's own record: its label, the pose clip's frame and each
path's value, in order. with --victim, the other fighter's record of the same frame follows on the
row: its label, place, damage and its own paths (the victim's freeze and launch beside the
attacker's refresh). --hop=N taps jump first and names the move N frames later, as hit_parity.py
plays an aerial from a short hop. a throw (throw_forward, throw_back, ...) is played by its grab
inputs. --placed puts the victim at the distance in the same batch as the
move, so the source's own push has no frames to part them before it begins. --dmg=N puts the victim at N% first (a launch's shape shows at
damage: at 0% most barely leave the floor). the frames run on the engine's own clock; each frame the fighter stood
still (a freeze) shows as a row repeated.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V  # noqa: E402
import trajectory as T  # noqa: E402
import hit_parity as H  # noqa: E402


def main():
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    victim = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--victim=")), None)
    hop = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--hop=")), None)
    dmg = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--dmg=")), None)
    cid, move, paths = args[0], args[1], args[2]
    at = float(args[3]) if len(args) > 3 else 34.0
    frames = int(args[4]) if len(args) > 4 else 60
    e = V.Ssf2Engine(f"probe-{cid}")
    if e.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or e.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(e, ("ANIM:ENTRANCE",))
    # the one to be hit faces the one playing the move, as hit_parity.py turns it (after a reset the
    # source turns it away: pikachu's thunder struck a victim's tail)
    e.tell_batch(["e p0.X = 0", f"e p1.X = {at}", "reset", "e p0.faceRight()", "e p1.faceLeft()"])
    V.standing(e, 1)
    H.victim_down(e, 1)
    H.push_settled(e, 1)
    if dmg is not None:
        e.tell(f"dmg p1 {dmg}")
    placed = None
    if "--placed" in sys.argv:
        # put at the distance asked for in the same batch as the move itself, so the source's push
        # has no frames between to part the two again (put back first and moved later, two bowsers
        # asked for 49 began a down tilt 78 apart)
        placed = V.ssf2_read(e, "e p0.X") + at
    # the record reads the paths off the fighter's clip, a dynamic object a string is written to
    e.tell(f'e p0.MC.peptideProbe = "{paths}"')
    if victim is not None:
        e.tell(f'e p1.MC.peptideProbe = "{victim}"')
    e.tick(1)
    e.tell("record")
    if hop is not None:
        e.tell(f"seq jump:1 none:{max(hop - 1, 1)}")
        e.tick(hop)
    if move.startswith("throw") and move in dict(H.MOVES):
        # a throw is played by its inputs, as hit_parity.py plays it (the source has no grab
        # button: a grab is shield with attack there), so the held and thrown fighter's own
        # values read through the grab and the let-go
        e.tell("seq " + V.fm_steps(dict(H.MOVES)[move].replace("grab:", "shield+attack:"), 1, source=True))
    elif placed is not None:
        e.tell_batch([f"e p1.X = {placed:.2f}", f"move {move} p0"])
    else:
        e.tell(f"move {move} p0")
    e.tick(frames)
    mark = e.tell("trace raw all" if victim is not None else "trace raw")
    e.tick(2)
    head = "label  frame  " + "  ".join(paths.split(";"))
    if victim is not None:
        head += "   |  victim: label  x  y  damage  " + "  ".join(victim.split(";"))
    print(head)
    row = None
    for line in e.log()[mark:].splitlines():
        if "\t" not in line:
            continue
        f = line.split("\t", 1)[1].split("|")
        if len(f) < 15:
            continue
        values = [v for v in f[14].split(";") if v != ""]
        if f[9].strip() == "0":
            if row is not None:
                print(row)
            row = f"{f[0]:6} {f[10]:>5}  " + "  ".join(values)
        elif row is not None:
            row += f"   |  {f[0]} {f[2]} {f[3]} {f[8]}  " + "  ".join(values)
            print(row)
            row = None
    if row is not None:
        print(row)
    e.close(final=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
