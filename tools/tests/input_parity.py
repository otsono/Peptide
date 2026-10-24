#!/usr/bin/env python3
"""input_parity.py -- does the same input play the same move on both engines?

`hit_parity.py` names each move on the source and presses its buttons here, so it compares what a
move does once it is playing. what a press turns INTO is a rule of its own: the source plays an up
attack pressed in a jump's squat as the grounded up tilt, and a converted fighter played its up air
there until that was ported. this presses the same input sequences on both engines, frame for frame
from standing, and lists the moves each one played, by this engine's names (the source's mapped
through the conversion's own table, `conversion_stats.json`). a sequence the two play differently is
marked `<<`.

usage: tools/tests/input_parity.py fighter [fighter ...] [--inputs="seq|seq|..."] [--show]
       a sequence is the harness's `a:N b` form in source frames (`jump:1 none:2 up+attack:1`);
       --show prints each engine's whole record (animation, frames) under a sequence that differs
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V  # noqa: E402
import trajectory as T  # noqa: E402
import hit_parity as H  # noqa: E402

# what each sequence asks: the attacks from standing, the specials, the grab, an attack pressed on
# each frame of a jump's squat and just after it, a walk and a crouch into an attack, a dash into one
INPUTS = [
    "attack", "right+attack", "up+attack", "down+attack",
    "special", "right+special", "up+special", "down+special",
    "shield+attack",
    "jump:1 up+attack", "jump:1 none:1 up+attack", "jump:1 none:2 up+attack", "jump:1 none:3 up+attack",
    "jump:1 attack", "jump:1 none:2 attack", "jump:1 right+attack", "jump:1 down+attack",
    "jump:1 up+special",
    # (a crouch and a dash off a double tap are not here: this engine takes them off the stick's
    # travel, which the harness's pressed buttons do not move, so its fighter stands and walks)
    "right:8 attack", "down:4 up+attack",
    # the way behind: a turn into an attack, a smash, a special
    "left+attack", "left:4 left+attack", "left:8 attack", "left+special", "left:4 left+special",
    # the shield's rolls and dodge
    "shield:4 shield+right:2", "shield:4 shield+left:2", "shield:4 shield+down:2",
    # how soon before the attack a direction still reads as a smash
    "right:1 right+attack", "right:2 right+attack", "right:3 right+attack", "right:5 right+attack",
    "up:1 up+attack", "up:3 up+attack", "down:1 down+attack", "down:3 down+attack",
]
# labels that are only the way into or out of a move, not one the input chose
PASSING = ("stand", "idle", "walk", "jump", "fall", "land", "crouch", "dash", "run", "skid", "turn",
           "heavyland", "helpless", "shield", "grab_hold", "grabbed", "pivot", "revival", "entrance",
           # a teeter or a hang at the edge a walk left reaches
           "ledge")


def moves_of(runs, rename=None):
    """the moves a record shows, in order, without the standing, walking and falling around them"""
    out = []
    for a, _ in runs:
        n = a.lower()
        # a move's own name first: one that begins like a passing label is still a move (the
        # source's crouch_attack is its down tilt)
        named = rename(a) if rename else None
        name = named if named is not None else a
        if name.lower().startswith(PASSING) or n.endswith(("_in", "_loop", "_out")) and n.split("_")[0] in ("jump", "fall", "walk", "run", "dash"):
            continue
        # a move's own pieces (its loop, its landing) read as the move
        for tail in ("_land", "_loop", "_attack", "_air", "_end", "_in", "_out", "_charge"):
            if name.endswith(tail) and len(name) > len(tail):
                name = name[: -len(tail)]
        # a combo's numbered pieces read as the one move (the source's jab is one label)
        name = name.rstrip("0123456789") or name
        if not out or out[-1] != name:
            out.append(name)
    return out


def play_source(cid, inputs):
    s = V.Ssf2Engine(f"inputs-ssf2-{cid}")
    got = {}
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        return None
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    # the middle of the stage, between where the match put the two (an x of 0 is not it on every
    # stage), read once: a reset does not put them back there
    mid = (V.ssf2_read(s, "e p0.X") + V.ssf2_read(s, "e p1.X")) / 2
    # the floor's height there, off the fighter's own record once it stands on it (the reflection
    # read of its height answers the spawn's): written with the x, so a fighter the last input left
    # on a platform is not left standing on it (lucario teetered at a platform's edge and walked off)
    s.tell("reset")
    s.tell_batch([f"e p0.X = {mid - 30:.1f}", "e p0.faceRight()"])
    V.standing(s, 1)
    s.tell("record")
    s.tick(4)
    import re
    ends = re.findall(r"end=\(([-\d.]+),([-\d.]+)\)", s.read_trace(s.tell("trace")) or "")
    floor = float(ends[-1][1]) if ends else None
    for inp in inputs:
        # at the middle, the other fighter well off to the left, both stood still
        s.tell("reset")
        s.tell_batch([f"e p0.X = {mid - 30:.1f}", f"e p1.X = {mid - 200:.1f}", "e p0.faceRight()"]
                     + ([f"e p0.Y = {floor:.2f}"] if floor is not None else []))
        V.standing(s, 1)
        # where it was put, read back: a reset the last input's state still holds over can land the
        # fighter back at a ledge after the write (lucario walked off one into its aerial)
        for _ in range(3):
            x = V.ssf2_read(s, "e p0.X")
            if x is None or abs(x - (mid - 30)) < 2:
                break
            s.tell_batch([f"e p0.X = {mid - 30:.1f}", "e p0.faceRight()"]
                         + ([f"e p0.Y = {floor:.2f}"] if floor is not None else []))
            V.standing(s, 1)
        s.tell("record")
        steps = V.fm_steps(inp.replace("grab", "shield+attack"), 1, source=True)
        m = s.tell(f"seq {steps}")
        s.await_seq(m, len(steps.split()))
        s.tick(60)
        got[inp] = V.trace_runs(s.read_trace(s.tell("trace")))
    s.close(final=True)
    return got


def play_port(cid, inputs):
    f = V.Engine(f"inputs-fm-{cid}")
    got = {}
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        return None
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(30)
    c0, c1 = "match.getCharacters()[0]", "match.getCharacters()[1]"
    mid = (float(f.eval(f"{c0}.getX()")) + float(f.eval(f"{c1}.getX()"))) / 2
    # the floor's height, read once standing on it, written with the x as on the source: one the last
    # input left hanging from a ledge or on a platform starts on the floor (falco walked off one)
    f.tell("reset")
    f.tell(f"e {c0}.setX({mid - 30 * V.SIZE_SCALE:.1f})")
    V.standing(f, 2)
    floor = float(f.eval(f"{c0}.getY()"))
    for inp in inputs:
        f.tell("reset")
        f.tell(f"e {c0}.setX({mid - 30 * V.SIZE_SCALE:.1f}); {c0}.setY({floor - 2:.1f}); {c0}.toState(CState.FALL); "
               f"{c1}.setX({mid - 200 * V.SIZE_SCALE:.1f}); {c0}.faceRight()")
        V.standing(f, 2)
        f.tell("record")
        # the port's grab is its own button; a tapped jump names its short hop (`fm_steps`)
        steps = V.fm_steps(inp.replace("shield+attack", "grab"), 2)
        m = f.tell(f"seq {steps}")
        f.await_seq(m, len(steps.split()))
        f.tick(120)
        got[inp] = V.trace_runs(f.read_trace(f.tell("trace")))
    f.close(final=True)
    return got


def one_fighter(cid, inputs):
    src = play_source(cid, inputs)
    port = play_port(cid, inputs)
    if src is None or port is None:
        print(f"{cid}: {'the source' if src is None else 'this engine'} never came up")
        return 1
    rename = lambda a: H.fm_name_of(cid, a)
    apart = 0
    print(f"{cid}:")
    for inp in inputs:
        a, b = moves_of(src.get(inp, []), rename), moves_of(port.get(inp, []))
        mark = "  " if a == b else "<<"
        apart += a != b
        print(f"  {mark} {inp:<34} source {' '.join(a) or '-':<40} here {' '.join(b) or '-'}")
        if a != b and "--show" in sys.argv:
            print(f"       source: {src.get(inp)}")
            print(f"       here:   {port.get(inp)}")
    print(f"  {len(inputs) - apart} of {len(inputs)} input(s) play the same move(s) on both")
    return 1 if apart else 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    given = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--inputs=")), None)
    inputs = given.split("|") if given else INPUTS
    if not args:
        print(__doc__)
        return 2
    worst = 0
    for cid in args:
        worst = max(worst, one_fighter(cid, inputs))
    return worst


if __name__ == "__main__":
    sys.exit(main())
