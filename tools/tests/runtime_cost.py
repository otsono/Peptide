#!/usr/bin/env python3
"""runtime_cost.py -- what each part of the runtime costs a converted fighter a frame, on Fraymakers.

the engine's scripts have no clock finer than a second, so nothing in them can time itself. this
measures from outside instead: the engine runs at `speed 8`, the engine process's processor time
over a long run gives the cost of a frame (its wall time moves only in whole screen refreshes), and each runtime function named is stubbed out on both
fighters in turn (and put back after), so the time a frame saves without it is what it costs. a
baseline is taken between every two so drift in the machine shows up next to the numbers, and each
measurement is the best of three runs (other work on the machine only ever adds time).

stubbing a part changes what the rest does (a fighter whose upkeep is gone can stand where it
would have moved), so the numbers are a guide to where the time goes, not an exact split.

usage: tools/tests/runtime_cost.py fighter [Name.path ...] [--frames=N] [--fight]
       (--fight: both fighters attack, one attack every 30 engine frames, instead of standing)
       (no names: the runtime's per-frame entry points; `a+b` stubs both together)
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import trajectory as T  # noqa: E402
import verify as V  # noqa: E402

DEFAULT = [
    "PeptideRuntime.update",
    "PeptideRuntime.sync",
    "PeptideRuntime.follow",
    "PeptideRuntime.settle",
    "PeptideRuntime.sourceFrame",
    "PeptideRuntime.kind.frame",
    "PeptideRuntime.kind.engineFrame",
    "MovieClip.tick",
    "Effects.frame",
    "Effects.show",
    "Rollback.frame",
]


# the attacks `--fight` cycles through
FIGHT = ("[CState.JAB, CState.TILT_FORWARD, CState.TILT_UP, CState.TILT_DOWN, CState.SPECIAL_NEUTRAL, "
         "CState.SPECIAL_SIDE, CState.STRONG_FORWARD_IN]")


def engine_pid(f):
    """the engine's process, as its session wrote it down beside itself"""
    try:
        kv = dict(l.split("=", 1) for l in open(os.path.join(f.dir, "meta"), encoding="utf8").read().splitlines() if "=" in l)
        return int(kv["engine_pid"].strip())
    except (OSError, KeyError, ValueError):
        return None


def cpu_seconds(pid):
    """a process's processor time so far (user and system), from `ps` ([[dd-]hh:]mm:ss.ss)"""
    out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    days, _, clock = out.rpartition("-")
    secs = 0.0
    for part in clock.split(":"):
        secs = secs * 60 + float(part)
    return secs + (int(days) * 86400 if days else 0)


def main():
    names = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not names:
        print(__doc__)
        return 2
    cid, parts = names[0], names[1:] or DEFAULT
    frames = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--frames=")), 4000)
    f = V.Engine(f"cost-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        print("the Fraymakers session never came up")
        return 1
    T.entrance_over(f, ("ANIM:INTRO",))
    f.tick(60)
    if "--fight" in sys.argv:
        # both fighters go through their attacks, one every 30 engine frames on the engine's own
        # timer (no input sequence, which holds the engine to real time), face to face
        f.tell("e match.getCharacters()[0].setX(-20); match.getCharacters()[1].setX(20)")
        for p in ("p0", "p1"):
            f.tell(f"e@{p} var __k = 0; var __s = {FIGHT}; "
                   "self.addTimer(30, -1, function() { __k = (__k + 1) % __s.length; self.toState(__s[__k]); }, { persistent: true })")
            f.tick(2)
    f.faster(8)

    engine = engine_pid(f)
    if engine is None:
        print("the engine's process is not known (no engine_pid beside the session)")
        return 1

    def frame_ms():
        # the engine's own processor time, not the wall clock: the engine waits for the screen
        # between ticks, so a frame's wall time only ever moves in whole refreshes (1/480 s a frame
        # at speed 8), and a part that costs less than that reads as nothing or as a whole one.
        # the best of three runs: the machine's other work only ever adds time
        best = None
        for _ in range(3):
            c0 = cpu_seconds(engine)
            f.tick(frames // 3)
            ms = (cpu_seconds(engine) - c0) * 1000 / (frames // 3)
            best = ms if best is None else min(best, ms)
        return best

    def on_both(expr):
        for p in ("p0", "p1"):
            m = f.tell(f"e@{p} {expr}")
            f.tick(2)
            err = [l for l in f.log()[m:].splitlines() if "ERR" in l]
            if err:
                return err[0].split("<<")[-1].strip()[:120]
        return None

    rows = []
    base = frame_ms()
    print(f"{cid} mirror, {frames} engine frames a run: {base:.2f} ms a frame with everything", flush=True)
    for i, part in enumerate(parts):
        # (`a+b+c` stubs them together: parts that each cost nothing alone can still add up)
        names = part.split("+")
        keeps = [f"__costKeep{i}_{j}" for j in range(len(names))]
        err = on_both("; ".join(f"{k} = {n}; {n} = function(?a, ?b, ?c, ?d) {{}}" for k, n in zip(keeps, names)))
        if err:
            print(f"  {part}: could not stub ({err})", flush=True)
            continue
        without = frame_ms()
        on_both("; ".join(f"{n} = {k}" for k, n in zip(keeps, names)))
        again = frame_ms()
        rows.append((part, (base + again) / 2 - without))
        print(f"  {part:36} {((base + again) / 2 - without):6.2f} ms a frame for two fighters "
              f"(baseline {again:.2f})", flush=True)
        base = again
    f.close(final=True)
    rows.sort(key=lambda r: -r[1])
    print("costliest first:", ", ".join(f"{p} {c:.2f}" for p, c in rows[:5]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
