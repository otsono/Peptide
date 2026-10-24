#!/usr/bin/env python3
"""state_sweep.py -- everything a player does that is NOT an attack, on the converted fighter.

The gates drive a fighter's attacks and specials. A match is mostly everything else: walking,
dashing, crouching, shielding, rolling, dodging, jumping twice, grabbing and throwing, hanging on
a ledge, getting hit, tumbling, dying and coming back. Those states are the engine's, and the
converter only supplies the animation each one shows. This puts the fighter through each one on
this engine and reports, per scenario:

  played     the animations the fighter showed, in order, with how long each ran
  missing    an animation the engine asked for that the fighter has no frames for
  stuck      the fighter never came back to standing (or idle) by the end
  errors     every script or engine error raised while it ran

A second fighter (the same character) stands beside it: it is what gets grabbed and what hits
back. Every wait is on the engine's own frames.

usage: tools/tests/state_sweep.py fighter [scenario ...]    (default: all)
       tools/tests/state_sweep.py --all
"""
import json, os, re, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
P0, P1 = "match.getCharacters()[0]", "match.getCharacters()[1]"
NEAR = 18.0   # the other fighter this far in front, in this engine's units


FRESH = {}


def fresh_match(eng):
    """a new match in the running engine, so a scenario that ended in a KO leaves nothing behind"""
    import time
    m = eng.tell(f"spawn {FRESH['cid']},{FRESH['cid']} {V.CHAR_STAGE_FM}")
    eng.reply(m, "LAUNCHED", 60, failfast=False)
    t0 = time.time()
    while "ANIM:INTRO" not in eng.log()[m:] and time.time() - t0 < 60:
        eng.next_line(1.0)
    T.entrance_over(eng, ("ANIM:INTRO",))
    eng.faster(1)
    # errors are counted from here: the match being rebuilt raises one of its own
    FRESH["mark"] = len(eng.log())


def put(eng, x0, x1, face_right=True):
    """both fighters on the floor where asked, p0 facing p1, in a match of their own"""
    fresh_match(eng)
    eng.tell("release all")
    eng.tell("reset")
    eng.tell(f"e {P0}.setX({x0}); {P1}.setX({x1})")
    V.standing(eng, 2)
    eng.tell(f"e {P0}.{'faceRight' if face_right else 'faceLeft'}(); {P1}.{'faceLeft' if face_right else 'faceRight'}()")
    eng.tick(2)


def watch(eng, who, frames):
    """the animation p0 showed on every engine frame for the next `frames` frames, off the
    engine's own recorder (a polled read is not frame-locked: the engine runs on between asks)"""
    eng.tick(frames)
    seq = []
    for a, n in V.trace_runs(eng.read_trace(eng.tell("trace"))):
        seq += [a] * n
    return seq


def runs(seq):
    out = []
    for a in seq:
        if out and out[-1][0] == a:
            out[-1][1] += 1
        else:
            out.append([a, 1])
    return out


def drive(eng, steps):
    m = eng.tell(f"seq {steps}")
    eng.await_seq(m, len(steps.split()))


# (name, setup, action) -- setup places the fighters, action drives and returns what to watch
def scenarios():
    s = {}

    def simple(steps, frames=90, near=None):
        def run(eng):
            put(eng, 0, 300 if near is None else near)
            # the inputs play one a frame while the fighter is watched, so a held state is seen
            # while it is held and not after it is let go
            eng.tell("record")
            eng.tell(f"seq {steps}")
            return P0, watch(eng, P0, frames)
        return run

    s["walk"] = simple("right:40", 70)
    s["dash"] = simple("right:2 none:2 right:30", 80)
    s["crouch"] = simple("down:40", 70)
    s["shield"] = simple("shield:50", 80)
    s["roll_forward"] = simple("shield:4 shield+right:4", 70)
    s["roll_back"] = simple("shield:4 shield+left:4", 70)
    s["spot_dodge"] = simple("shield:4 shield+down:4", 60)
    s["jump_double"] = simple("jump:3 none:16 jump:3", 120)
    s["air_dodge"] = simple("jump:3 none:10 shield:3", 110)
    s["fast_fall"] = simple("jump:3 none:24 down:10", 110)
    s["taunt"] = simple("taunt:3", 150)
    s["grab_whiff"] = simple("grab:3", 70)
    for d, key in (("forward", "right"), ("back", "left"), ("up", "up"), ("down", "down")):
        s[f"throw_{d}"] = simple(f"grab:3 none:14 {key}:4", 120, near=NEAR)
    s["grab_pummel"] = simple("grab:3 none:14 attack:2 none:10 attack:2", 90, near=NEAR)

    def hit(dmg, attacker_steps, frames):
        def run(eng):
            # p1 does the hitting: its own control is not driven, so it is told the move
            put(eng, 0, NEAR, face_right=True)
            eng.tell(f"e {P0}.setDamage({dmg})")
            eng.tell("record")
            eng.tell(f"e {P1}.toState(CState.{attacker_steps})")
            return P0, watch(eng, P0, frames)
        return run

    s["hit_light"] = hit(0, "JAB", 90)
    s["hit_hard"] = hit(150, "STRONG_FORWARD_ATTACK", 150)

    def ledge(eng):
        # walk off the right side of the main floor and hold back toward it on the way down, so
        # the lip is caught, then climb
        put(eng, 0, -300)
        eng.tell("record")
        eng.tell("seq right:110 left:90 none:60 up:3")
        return P0, watch(eng, P0, 300)
    s["ledge"] = ledge

    def ko(eng):
        # walked off the right edge and held away: falls through the blast zone, then comes back
        put(eng, 0, -300)
        eng.tell("record")
        eng.tell("seq right:600")
        return P0, watch(eng, P0, 700)
    s["ko_respawn"] = ko
    return s


def one_fighter(cid, names):
    project = next(iter(__import__("glob").glob(os.path.join(ROOT, "characters", cid, "*.fraytools"))), None)
    anims = set()
    for p in __import__("glob").glob(os.path.join(ROOT, "characters", cid, "library", "entities", "*.entity")):
        try:
            d = json.load(open(p))
        except Exception:
            continue
        if any(a.get("name") == "stand" for a in d.get("animations", [])):
            anims |= {a["name"] for a in d["animations"]}
    FRESH["cid"] = cid
    f = V.Engine(f"states-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.faster(1)
    report = {"fighter": cid, "scenarios": {}}
    for name, run in scenarios().items():
        if names and name not in names:
            continue
        before = len(f.log())
        try:
            who, seq = run(f)
        except Exception as e:  # a scenario the harness could not stage is reported, not fatal
            report["scenarios"][name] = {"harness": repr(e)}
            continue
        body = f.log()[max(before, FRESH.get("mark", 0)):]
        errs = sorted({l.strip()[:220] for l in body.splitlines()
                       if re.search(r"SCRIPTERR|Interpret Error|Invalid|not found|Null access|Exception", l)})
        seen = [a for a in seq if a]
        missing = sorted({a for a in seen if anims and a not in anims})
        tail = [a for a in seq[-5:] if a]
        stuck = bool(tail) and not any(t.startswith(("stand", "idle", "crouch", "walk", "fall", "revival", "shield")) for t in tail) and name not in ("ko_respawn",)
        report["scenarios"][name] = {"played": runs(seq), "missing": missing, "stuck": stuck, "errors": errs}
    f.close(final=True)
    os.makedirs(OUT, exist_ok=True)
    json.dump(report, open(os.path.join(OUT, f"states-{cid}.json"), "w"), indent=1)
    bad = 0
    print(f"== {cid}")
    for name, r in report["scenarios"].items():
        if "harness" in r:
            print(f"  {name:14} could not stage: {r['harness'][:120]}")
            continue
        flags = []
        if r["missing"]:
            flags.append(f"MISSING {r['missing']}")
        if r["stuck"]:
            flags.append("STUCK")
        if r["errors"]:
            flags.append(f"{len(r['errors'])} ERROR(S): {r['errors'][0][:140]}")
        bad += bool(flags)
        played = " ".join(f"{a}:{n}" for a, n in r["played"] if a)
        print(f"  {name:14} {played[:150]}" + ("\n" + " " * 17 + " | ".join(flags) if flags else ""))
    return 1 if bad else 0


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if "--all" in sys.argv:
        roster = sorted(json.load(open(os.path.join(OUT, "ssf2-data.json")))["character"])
        worst = 0
        for cid in (args or roster):
            worst = max(worst, one_fighter(cid, []))
        return worst
    if not args:
        print(__doc__)
        return 2
    return one_fighter(args[0], args[1:])


if __name__ == "__main__":
    sys.exit(main())
