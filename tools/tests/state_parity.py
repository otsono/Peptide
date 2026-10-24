#!/usr/bin/env python3
"""state_parity.py -- how long each thing a fighter does besides attacking lasts, on both engines.

A roll that runs twice as long, a spot dodge that ends early, a throw that holds the other fighter
too long: the gates never drive any of them, and a player feels every one. This drives the same
inputs on each engine (the other fighter set down in front where a grab needs it), reads each
engine's own per-frame record, and compares the length of the state the input starts, in source
frames (this engine runs two for each).

usage: tools/tests/state_parity.py fighter [scenario ...]
       tools/tests/state_parity.py --all
"""
import json, os, re, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
TOL = 0.2
# (scenario, inputs in SOURCE frames, the other fighter in front?, source animations that count)
SCENARIOS = [
    ("roll_forward", "shield:3 shield+right:3", False, ["dodgeroll", "roll"]),
    ("roll_back", "shield:3 shield+left:3", False, ["dodgeroll", "roll"]),
    ("spot_dodge", "shield:3 shield+down:3", False, ["sidestep", "dodge"]),
    ("grab_whiff", "grab:2", False, ["grab"]),
    ("throw_forward", "grab:2 none:8 right:3", True, ["throw_forward"]),
    ("throw_back", "grab:2 none:8 left:3", True, ["throw_back"]),
    ("throw_up", "grab:2 none:8 up:3", True, ["throw_up"]),
    ("throw_down", "grab:2 none:8 down:3", True, ["throw_down"]),
    ("taunt", "taunt:2", False, ["taunt", "taunt_up", "taunt_down"]),
]


def steps_for(steps, per, ssf2=False):
    # the source has no grab button: a grab is shield with attack there
    if ssf2:
        steps = steps.replace("grab:", "shield+attack:")
    return " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" for t in steps.split())


def fresh(eng, cid, ssf2):
    if ssf2:
        return
    m = eng.tell(f"spawn {cid},{cid} {V.CHAR_STAGE_FM}")
    eng.reply(m, "LAUNCHED", 60, failfast=False)
    t0 = time.time()
    while "ANIM:INTRO" not in eng.log()[m:] and time.time() - t0 < 60:
        eng.next_line(1.0)
    T.entrance_over(eng, ("ANIM:INTRO",))
    eng.faster(1)


def run(eng, cid, ssf2, steps, near):
    per = 1 if ssf2 else 2
    fresh(eng, cid, ssf2)
    eng.tell("release all")
    eng.tell("reset")
    if ssf2:
        # source units: the other fighter a short step in front, both facing each other, both on
        # the floor: written at its height as well, the other one a throw sent high is not left to
        # fall back over the next grab (the source's up and down throws closed on nothing)
        floor = getattr(eng, "floor_y", None)
        if floor is None:
            eng.tell("e p0.X = 0")
            V.standing(eng, per)
            eng.tell("record")
            eng.tick(4)
            ends = re.findall(r"end=\(([-\d.]+),([-\d.]+)\)", eng.read_trace(eng.tell("trace")) or "")
            floor = eng.floor_y = float(ends[-1][1]) if ends else None
        eng.tell_batch(["e p0.X = 0", f"e p1.X = {14 if near else 240}"]
                       + ([f"e p0.Y = {floor:.2f}", f"e p1.Y = {floor:.2f}"] if floor is not None else []))
    else:
        eng.tell(f"e match.getCharacters()[0].setX(0); match.getCharacters()[1].setX({18 if near else 300})")
    V.standing(eng, per)
    # the other fighter, reset out of the last throw, can still be in the air falling to where it
    # was put: a grab reaching for it then closes on nothing (the source's up and down throws read
    # as never played, the scenarios after a throw that sent it high)
    if near:
        eng.tick(30 * per)
    if ssf2:
        eng.tell_batch(["e p0.faceRight()", "e p1.faceLeft()"])
    else:
        eng.tell("e match.getCharacters()[0].faceRight(); match.getCharacters()[1].faceLeft()")
    eng.tick(2 * per)
    eng.tell("record")
    eng.tell(f"seq {steps_for(steps, per, ssf2)}")
    eng.tick(150 * per)
    return V.trace_runs(eng.read_trace(eng.tell("trace")))


def length_of(runs, names):
    """source frames the first run of any of `names` lasted, and which one it was"""
    for a, n in runs:
        if a in names:
            return a, n
    return None, 0


def one_fighter(cid, names):
    try:
        table = json.load(open(os.path.join(ROOT, "characters", cid, "conversion_stats.json")))["ssf2_to_fm_anim"]
    except (OSError, KeyError):
        table = {}
    todo = [s for s in SCENARIOS if not names or s[0] in names]
    s = V.Ssf2Engine(f"stateparity-ssf2-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2]) is None or s.wait(("ANIM:",), 120) is None:
        print("the SSF2 session never came up")
        return 1
    T.entrance_over(s, ("ANIM:ENTRANCE",))
    s.faster(1)
    src = {name: run(s, cid, True, steps, near) for name, steps, near, _ in todo}
    s.close(final=True)
    f = V.Engine(f"stateparity-fm-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.faster(1)
    port = {name: run(f, cid, False, steps, near) for name, steps, near, _ in todo}
    f.close(final=True)
    report, bad = {"fighter": cid, "scenarios": {}}, 0
    print(f"== {cid}")
    for name, steps, near, src_names in todo:
        a, n_src = length_of(src[name], src_names)
        fm_names = {table.get(x, x) for x in src_names} | ({table.get(a, a)} if a else set())
        # this engine plays a taunt as its emote
        if name == "taunt":
            fm_names.add("emote")
        b, n_fm = length_of(port[name], fm_names)
        n_fm_src = n_fm / 2.0
        row = {"source": [a, n_src], "port": [b, n_fm], "source_runs": src[name][:8], "port_runs": port[name][:8]}
        verdict = ""
        if not a:
            verdict = "the source never played it"
        elif not b:
            verdict = f"the port never played it (source {a} {n_src}f)"
            bad += 1
        elif abs(n_fm_src - n_src) > max(2, TOL * n_src):
            verdict = f"LENGTH: source {a} {n_src}f, port {b} {n_fm_src:.0f}f"
            bad += 1
        row["verdict"] = verdict
        report["scenarios"][name] = row
        print(f"  {name:14} source {a}:{n_src}  port {b}:{n_fm_src:.0f}  {verdict}")
    os.makedirs(OUT, exist_ok=True)
    json.dump(report, open(os.path.join(OUT, f"stateparity-{cid}.json"), "w"), indent=1)
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
