#!/usr/bin/env python3
"""soak.py -- play a converted fighter the way a person does, and collect what breaks.

The gates play each move on its own, from standing, into empty air. A match strings moves
together, lands them on someone, gets hit back, grabs and is grabbed, jumps off the edge and comes
back. This drives seeded random inputs (a mix weighted toward attacks and specials, with the other
fighter kept close so hits land both ways) for a few thousand engine frames and reports every
distinct script error the engine raised, with where it came from and how often.

usage: tools/tests/soak.py fighter [fighter ...] [--frames=N] [--seed=S]
       tools/tests/soak.py --all | --changed [fighter ...]
  several fighters share one engine session (a new match each). --changed plays only the ones whose
  converted scripts changed since their last soak, or that reported something then.
"""
import collections, json, os, random, re, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V
import trajectory as T

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build", "verify")
DIRS = ["", "right", "left", "up", "down"]
BUTTONS = [("attack", 5), ("special", 5), ("jump", 2), ("shield", 1), ("grab", 1), ("", 2)]
P0, P1 = "match.getCharacters()[0]", "match.getCharacters()[1]"
SPEED = 8


def opt(name, default):
    for a in sys.argv:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def pick(rng):
    b = rng.choices([b for b, _ in BUTTONS], weights=[w for _, w in BUTTONS])[0]
    d = rng.choice(DIRS)
    ctl = "+".join(x for x in (d, b) if x) or "none"
    return f"{ctl}:{rng.choice([2, 4, 8, 16])} none:{rng.choice([4, 10, 20, 40])}"


def origin(line):
    m = re.search(r"Interpret Error:\s*([^:]+:[^:]+(?::frame\d+)?)", line) or re.search(r"\(origin: ([^)]+)\)", line)
    return m.group(1) if m else "?"


def content_hash(cid):
    """what the fighter's packed build was made from, so a batch can skip one that has not changed"""
    import hashlib
    h = hashlib.sha256()
    lib = os.path.join(ROOT, "characters", cid, "library")
    for root, _, files in sorted(os.walk(lib)):
        for name in sorted(files):
            if name.endswith((".hx", ".entity")):
                h.update(name.encode())
                h.update(open(os.path.join(root, name), "rb").read())
    return h.hexdigest()


def arrive(cid, eng):
    """a match with this fighter: a new one in the running engine when there is one (no relaunch,
    which is most of what a fighter costs), else a fresh boot. Answers (engine, log mark)"""
    import time
    if eng is not None and not eng.crashed():
        mark = eng.tell(f"spawn {cid},{cid} {V.CHAR_STAGE_FM}")
        if eng.reply(mark, "LAUNCHED", 60, failfast=False) is not None:
            t0 = time.time()
            while "ANIM:INTRO" not in eng.log()[mark:] and time.time() - t0 < 60:
                eng.next_line(1.0)
            T.entrance_over(eng, ("ANIM:INTRO",))
            eng.faster(SPEED)
            return eng, mark
        eng.close(final=True)
    f = V.Engine(f"soak-{cid}")
    if f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM]) is None:
        return None, 0
    T.entrance_over(f, ("ANIM:INTRO",))
    f.faster(SPEED)
    return f, 0


def one_fighter(cid, frames, seed, eng=None, keep=False):
    rng = random.Random(f"{cid}:{seed}")
    f, boot_mark = arrive(cid, eng)
    if f is None:
        print(f"{cid}: the Fraymakers session never came up")
        return (1, None) if keep else 1
    start = len(f.log())
    played = 0
    # a projectile left in its dying state, named the moment the engine says one exists (by the
    # end of play it may be gone)
    stuck, seen_warn = [], start
    def ask(q):
        m = f.tell("e " + q)
        return str(f.reply(m, "E:", 20, failfast=False) or "").replace("E:", "").replace("<<", "").strip()
    while played < frames:
        # the other fighter kept in reach, facing the driven one, now and then made to swing back
        if rng.random() < 0.25:
            f.tell(f"e {P1}.setX({P0}.getX() + {rng.choice([25, -25])})")
        if rng.random() < 0.15:
            f.tell(f"move {rng.choice(['jab1', 'tilt_forward', 'tilt_up', 'special_neutral', 'aerial_neutral'])} p1")
        chunk = " ".join(pick(rng) for _ in range(4))
        # at full speed: this engine plays a sequence one step per TICK (eight frames at speed 8),
        # and random play needs no frame-exact taps, so each hold and wait is given in ticks and
        # the soak does not drop to real time for it (which made a fighter cost a minute)
        steps = [(t.split(':')[0], max(1, round(int(t.split(':')[1]) / SPEED))) for t in chunk.split()]
        f.tell_batch(["seq " + " ".join(f"{c}:{n}" for c, n in steps)])
        n = sum(k for _, k in steps) * SPEED
        f.tick(n)
        played += n
        if f.crashed():
            break
        log_now = f.log()
        # the engine names the animation of a projectile that has gone twenty seconds without a
        # state change (its text says dying; it is any state): the one playing it is named here
        warned = [l.rsplit("animation ", 1)[-1].rstrip(". ") for l in log_now[seen_warn:].splitlines() if "check out the projectile" in l]
        if warned and len(stuck) < 6:
            seen_warn = len(log_now)
            cnt = ask("match.getProjectiles().length")
            for i in range(min(int(cnt) if cnt.isdigit() else 0, 12)):
                p = f"match.getProjectiles()[{i}]"
                if ask(f"{p}.getAnimation()") in warned:
                    stuck.append(" ".join(ask(f"{p}.{g}") for g in ("exports.__linkageId", "getAnimation()", "getCurrentFrame()", "getTotalFrames()", "finalFramePlayed()", "getOwner().getResource().getId()")))
    body = f.log()[start:]
    # a script that does not parse says so while the match loads, before play starts; its frames
    # then fail on every name it declares, which reads like scattered bugs rather than one
    boot = f.log()[boot_mark:start]
    unparsed = [l.strip() for l in boot.splitlines() if "Parse Error" in l or "Could not parse" in l]
    crashed = f.crashed()
    # what took the engine down, and what the fighter was doing just before: the session's
    # files are the next fighter's by the time anyone reads them
    why = f.crash_reason() if crashed else ""
    # the engine's own account of the crash, which goes when the session closes
    crash_log = ""
    if crashed:
        for name in ("crash.log", "boot.log"):
            path = os.path.join(f.dir, name)
            if os.path.exists(path):
                crash_log += f"== {name}\n" + open(path, encoding="utf8", errors="replace").read()[-6000:]
    last = [l for l in body.splitlines() if l.startswith("<<")][-25:] if crashed else []
    if crashed or not keep:
        f.close(final=True)
        f = None
    # a console ask of the harness's own (origin `hscript`) is not the fighter's
    errs = unparsed + [l.strip() for l in body.splitlines()
            if ("SCRIPTERR" in l or "Interpret Error" in l) and "(origin: hscript)" not in l and "hscript:1:" not in l
            # the engine's note on a projectile that has gone twenty seconds without changing
            # state, dying or not: a long-lived one is no error (named below as long-lived)
            and "DESTROYING state" not in l]
    by = collections.Counter(re.sub(r"\d+\.\d+", "N", e[:200]) for e in errs)
    report = {"fighter": cid, "frames": played, "crashed": crashed, "crash_reason": why, "before_crash": last,
              "content": content_hash(cid), "stuck_dying": stuck, "crash_log": crash_log,
              "errors": [{"n": n, "line": l, "origin": origin(l)} for l, n in by.most_common()]}
    os.makedirs(OUT, exist_ok=True)
    json.dump(report, open(os.path.join(OUT, f"soak-{cid}.json"), "w"), indent=1)
    print(f"== {cid}: {played} frames, {len(errs)} error line(s), {len(by)} distinct" + (f" | ENGINE CRASHED{why[:200]}" if crashed else ""))
    for l, n in by.most_common(8):
        print(f"  x{n:<4} {l[:190]}")
    for st in stuck:
        print(f"  long-lived: {st}  (linkage, animation, frame, of, last frame played)")
    code = 1 if (errs or crashed) else 0
    return (code, f) if keep else code


def needs_soak(cid):
    """changed since its last soak, or it had something to report then"""
    try:
        r = json.load(open(os.path.join(OUT, f"soak-{cid}.json")))
    except Exception:
        return True
    return r.get("content") != content_hash(cid) or r.get("crashed") or bool(r.get("errors"))


def batch(cids, frames, seed):
    """several fighters through ONE engine session, a new match each"""
    eng, worst = None, 0
    for cid in cids:
        code, eng = one_fighter(cid, frames, seed, eng, keep=True)
        worst = max(worst, code)
    if eng is not None:
        eng.close(final=True)
    return worst


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    frames = int(opt("frames", "3000"))
    seed = opt("seed", "1")
    if "--all" in sys.argv or "--changed" in sys.argv or len(args) > 1:
        roster = sorted(json.load(open(os.path.join(OUT, "ssf2-data.json")))["character"])
        picked = args or roster
        if "--changed" in sys.argv:
            picked = [c for c in picked if needs_soak(c)]
            print(f"soaking {len(picked)}: {' '.join(picked)}")
        return batch(picked, frames, seed)
    if not args:
        print(__doc__)
        return 2
    return one_fighter(args[0], frames, seed)


if __name__ == "__main__":
    sys.exit(main())
