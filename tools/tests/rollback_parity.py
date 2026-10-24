#!/usr/bin/env python3
"""rollback_parity.py -- does a converted fighter play the same when the match rolls back?

an online match keeps saving its state and, when a remote input arrives late, puts the state back
some frames and plays them again. anything a fighter's scripts keep outside the engine's saved
state is then played twice: a counter counts double, a flag set in a frame that is replayed stays
set, and the fighter drifts from where the same inputs take it offline.

this plays one seeded run of inputs twice on Fraymakers: in an ordinary match, then in a local
match that rolls back the way an online one does (`netcode = 3` in the match settings, no network
involved), off a record the fighter's own script keeps of each frame of the match, and compares
what it played: the same animations in the same order, each move the same length (the time spent
standing or walking between inputs is the input's, not the fighter's, and is left out).

the exact check needs no offline run at all: the local match replays frames whose inputs it
already had, so each one must come out as it did the first time. the fighter's script compares
every replayed frame (the engine's animation, frame and place, and the runtime's own clip state)
against its first playing, and any that differs is state a rollback did not put back. the fighter has to be
converted and packed on the runtime's path first.

usage: tools/tests/rollback_parity.py fighter [--frames=N] [--seed=S] [--baseline] [--engine-record]
  --baseline  also plays the ordinary match a second time: what differs between two ordinary runs
              is the harness's own noise, not rollback
  --engine-record  read the engine's own per-frame record instead (for a stock fighter, as the
              control: what the engine and the harness do under rollback with no script of ours)
results: build/verify/rollback-<fighter>.json
"""
import json, os, random, sys, tempfile
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
DIRS = ["", "right", "left", "up", "down"]
BUTTONS = [("attack", 5), ("special", 5), ("jump", 2), ("shield", 1), ("grab", 1), ("", 2)]
SPEED = 8
P0 = "match.getCharacters()[0]"


def opt(name, default):
    for a in sys.argv:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def plan(cid, seed, frames):
    """the inputs, as (control, ticks) steps: the same for every run"""
    rng = random.Random(f"{cid}:{seed}")
    # a moment of standing first, so the first input is not the frame the record starts on
    steps, n = [("none", 6)], 6 * SPEED
    while n < frames:
        b = rng.choices([b for b, _ in BUTTONS], weights=[w for _, w in BUTTONS])[0]
        d = rng.choice(DIRS)
        # held two ticks at least: a match that rolls back runs its frames unevenly against the
        # harness's (it waits on the other side's inputs), and a one-tick tap can fall between them
        hold = rng.choice([2, 3])
        rest = rng.choice([1, 2, 4, 6])
        steps.append(("+".join(x for x in (d, b) if x) or "none", hold))
        steps.append(("none", rest))
        n += (hold + rest) * SPEED
    return steps


def settings_with(netcode):
    """the shared match settings with the netcode set, as a file the session reads"""
    text = open(os.path.join(ROOT, "match_settings.conf"), encoding="utf8").read()
    lines = [l for l in text.splitlines() if not l.strip().startswith("netcode")]
    lines.append(f"netcode = {netcode}")
    fd, path = tempfile.mkstemp(suffix=".conf")
    with os.fdopen(fd, "w") as f:
        f.write("\n".join(lines) + "\n")
    return path


def run(cid, steps, netcode, tag):
    """one match: the whole run of inputs started in the same frame as the engine's own per-frame
    record, and read back from it (a console read lands on no particular frame)"""
    os.environ["PEPTIDE_MATCH_SETTINGS"] = settings_with(netcode)
    f = V.Engine(f"rollback-{cid}-{tag}")
    try:
        if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
            return None
        T.entrance_over(f, ("ANIM:INTRO",))
        f.faster(SPEED)
        f.tell("reset")
        f.tick(SPEED * 4)
        # the fighter's own script files the fighter's state under the match's frame, which a
        # rollback takes back: a frame played again overwrites what it first recorded, so what is
        # read is the match as it stood once every frame was settled (the engine's per-frame
        # record would list the replayed frames too)
        if "--engine-record" in sys.argv:
            # a fighter with no script of ours (a stock one, the control): the engine's own record,
            # which also lists the frames a rollback plays again, so only what played, in order
            f.tell_batch(["record", "seq " + " ".join(f"{c}:{n}" for c, n in steps)])
            f.tick(sum(n for _, n in steps) * SPEED + SPEED)
            return {"rows": [[a, x, y, 0] for a, x, y in V.raw_trace_rows(f)], "errors": []}
        m = f.tell(f"e {P0}.exports.__traceStart()")
        if f.reply(m, "E:", 20, failfast=False) is None:
            return None
        f.tell_batch(["seq " + " ".join(f"{c}:{n}" for c, n in steps)])
        f.tick(sum(n for _, n in steps) * SPEED + SPEED)
        if f.crashed():
            return {"rows": [], "crashed": f.crash_reason()[:300], "errors": []}
        def ask(q):
            m = f.tell(f"e {q}")
            return str(f.reply(m, "E:", 180, failfast=False) or "").replace("<< ", "").replace("E:", "", 1).strip()
        n = int(ask(f"{P0}.exports.__traceLength()") or 0)
        rows = []
        for at in range(0, n, 60):
            for rec in ask(f"{P0}.exports.__traceRead({at}, 60)").split(";"):
                parts = rec.split("|")
                rows.append([parts[0], float(parts[2]), float(parts[3]), int(parts[1])] + parts[4:] if len(parts) >= 4 else ["?", 0.0, 0.0, 0])
        replays = ask(f"{P0}.exports.__traceReplays()")
        errors = [l.strip() for l in f.log().splitlines() if ("SCRIPTERR" in l or "Interpret Error" in l) and "(origin: hscript)" not in l]
        return {"rows": [list(r) for r in rows], "errors": errors[:40], "replays": replays}
    finally:
        f.close(final=True)
        os.environ.pop("PEPTIDE_MATCH_SETTINGS", None)


# what the fighter does between inputs, whose length is the input's own timing rather than the
# fighter's: a rolled-back match takes inputs a frame or two apart from an offline one
NEUTRAL = ("stand", "walk", "idle", "crouch", "dash", "run", "skid", "jab", "shield")


def runs(rows):
    """the animations played, in order, with how many frames each ran"""
    out = []
    for r in rows:
        if out and out[-1][0] == r[0]:
            out[-1][1] += 1
        else:
            out.append([r[0], 1])
    return out


def differ(a, b):
    """where two runs part: the first animation played in a different order, and every move that
    ran a different length (more than a frame off), as (index, what, a, b)"""
    ra, rb = runs(a["rows"]), runs(b["rows"])
    out = []
    for i, (x, y) in enumerate(zip(ra, rb)):
        if x[0] != y[0]:
            out.append((i, "animation", x[0], y[0]))
            break
        last = i == len(ra) - 1 or i == len(rb) - 1
        if not last and not x[0].startswith(NEUTRAL) and abs(x[1] - y[1]) > 1:
            out.append((i, f"{x[0]} length", x[1], y[1]))
    return out


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if not args:
        print(__doc__)
        return 2
    cid = args[0]
    frames = int(opt("frames", "2400"))
    steps = plan(cid, opt("seed", "1"), frames)
    offline = run(cid, steps, 0, "offline")
    rolled = run(cid, steps, 3, "rollback")
    if offline is None or rolled is None:
        print(f"{cid}: a session never came up")
        return 1
    for name, r in (("offline", offline), ("rollback", rolled)):
        if not r["rows"]:
            print(f"{cid}: the {name} run recorded nothing" + (f" (crashed: {r.get('crashed')})" if r.get("crashed") else ""))
            return 1
    report = {"fighter": cid, "frames": len(offline["rows"]), "rollback_errors": rolled["errors"],
              "offline_errors": offline["errors"]}
    if "--engine-record" in sys.argv:
        seq = lambda rows: [k for k, n in ((k, len(list(g))) for k, g in __import__("itertools").groupby(r[0] for r in rows)) if n > 2]
        so, sr = seq(offline["rows"]), seq(rolled["rows"])
        first = next((i for i, (x, y) in enumerate(zip(so, sr)) if x != y), None)
        print(f"{cid}: {len(so)} animations offline, {len(sr)} under rollback; first different at {first}")
        print("  offline:", " ".join(so[:30]))
        print("  rollback:", " ".join(sr[:30]))
        return 0 if first is None else 1
    d = differ(offline, rolled)
    report["rollback_differs"] = [list(x) for x in d]
    # the exact check: every frame the match played again, against how it first came out
    report["replays"] = rolled.get("replays")
    print(f"{cid}: frames played again under rollback: {rolled.get('replays')}")
    report["offline_rows"], report["rollback_rows"] = offline["rows"], rolled["rows"]
    if "--baseline" in sys.argv:
        again = run(cid, steps, 0, "offline2")
        report["baseline_differs"] = [list(x) for x in differ(offline, again)] if again else None
    os.makedirs(OUT, exist_ok=True)
    json.dump(report, open(os.path.join(OUT, f"rollback-{cid}.json"), "w"), indent=1)
    bad = sorted({i for i, *_ in d})
    print(f"{cid}: {len(runs(offline['rows']))} animations played offline, {len(bad)} differ under rollback"
          + (f" (first at the {bad[0]}th)" if bad else "")
          + (f"; baseline {len({i for i, *_ in report['baseline_differs']})} differ" if report.get("baseline_differs") is not None else ""))
    for i, k, a, b in d[:12]:
        print(f"  #{i} {k}: offline {a} / rollback {b}")
    if rolled["errors"]:
        print(f"  {len(rolled['errors'])} script error line(s) under rollback, first: {rolled['errors'][0][:160]}")
    exact = str(rolled.get("replays") or "")
    return 1 if (" 0 differ" not in exact) else 0


if __name__ == "__main__":
    sys.exit(main())
