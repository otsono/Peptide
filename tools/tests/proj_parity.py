#!/usr/bin/env python3
"""proj_parity.py -- what a fighter's projectiles DO, on both engines, side by side.

the gates play each move and read the fighter's own animation; the soak looks for script errors.
neither tells a boomerang that flies off forever from one that comes back to be caught: the move
plays and nothing throws. this fires each special on both engines with nobody near and reads its
projectiles every frame, on each engine's own clock: SSF2 through the per-frame record its
injected recorder writes (the count and the first projectile's position ride on every line), and
Fraymakers through a counter the fighter's own script keeps for the length of the move. then it
compares the shape of what happened, in source frames:

  first  the frame the first projectile appears
  life   how long projectiles were out
  most   the most out at once
  back   whether the first one came back (ended within a third of its farthest distance)
  plays  how many times the thrower played the move (a catch plays it again)

usage: tools/tests/proj_parity.py fighter [fighter ...] [--moves=special_side,special_neutral] [--fresh]
                                  [--check] [--save] [--left]

--fresh converts each fighter from the source engine's own data and packs it first; without it
the package already installed is the one measured
a row whose shape differs is marked `<<`. a move with no projectile on either side is left out.

--check holds the run to `proj_parity_baseline.json` (beside this file): a move it lists as
matching that differs now is a regression, and the run fails on it (a move that differs and is
not listed is reported, not failed). --save writes the moves that match this run into it.

--left plays every move with the thrower facing left (its moves are kept in the baseline apart,
as `fighter/move<`).
"""
import json, os, re, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V, trajectory as T

MOVES = ["special_neutral", "special_side", "special_up", "special_down"]
FRAMES = 180   # source frames watched per move
# a source pixel in this engine's units (the converter's `__sp`)
SIZE_SCALE = 1.3
IDLE = {"stand", "idle", "walk", "fall", "fall_loop", "land", "land_light", "jump", "revival"}


def opt(name, default):
    for a in sys.argv:
        if a.startswith(f"--{name}="):
            return a.split("=", 1)[1]
    return default


def plays_of(runs):
    move = next((a for (a, _n) in runs if a not in IDLE), None)
    return sum(1 for (a, _n) in runs if a == move) if move else 0


def summary(first, last, most, far, end, plays, dy=None):
    if first is None:
        return {"most": 0, "plays": plays}
    out = {"first": first, "life": last - first + 1, "most": most,
           "back": far > 0 and end <= far / 3, "plays": plays}
    if dy is not None:
        out["dy"] = round(dy)
    return out


# where the other fighter stands, in the source's pixels: far enough that a shot's reach is its own,
# near enough to stay on the stage (a homing shot chases whoever is there, on both engines alike)
AWAY = 500
# where the thrower stands, in the source's pixels
HOME = -250


def fresh(cid):
    """the fighter converted from the data the source engine itself runs (the installed game's,
    as verify.py converts it, not the corpus copy, which can be another build of the fighter)
    and packed, so what is compared is the same fighter on both sides"""
    src = V.source_ssf(cid, "character")
    code, out, _ = V.run([V.BIN, "convert", src, "-n", cid, "-o", os.path.join(V.ROOT, "characters"), "-y"], V.CONVERT_TIMEOUT_S)
    if code != 0:
        raise SystemExit(f"{cid}: convert failed: {out.strip().splitlines()[-1] if out.strip() else ''}")
    code, out, _ = V.run([V.BIN, "pack", "--project", os.path.join(V.ROOT, "characters", cid, f"{cid}.fraytools")], 300)
    if code != 0:
        raise SystemExit(f"{cid}: pack failed: {out.strip().splitlines()[-1] if out.strip() else ''}")
    print(f"[{cid}] converted from {src} and packed", file=sys.stderr)


# `--left`: the thrower facing left. a projectile thrown facing left is the half the right-facing
# run never sees (its x speed reads and writes along the throw on the source)
LEFT = "--left" in sys.argv


def reset(eng, is_ssf2):
    # `--left` mirrors the whole layout: the thrower on the other side, facing the other fighter
    # by a tap left. turned away instead, its side specials walked it off the source's ledge
    home, there, face = (-HOME, -(HOME + AWAY), "left") if LEFT else (HOME, HOME + AWAY, "right")
    eng.faster(8)
    eng.tell("release all"); eng.tell("reset"); eng.tick(20)
    # the same spot on both stages: a shot that hugs terrain meets the same floor and edges
    if is_ssf2:
        eng.tell(f"e p0.X = {home}"); eng.tell(f"e p1.X = {there}")
    else:
        eng.tell(f"e match.getCharacters()[0].setX({home * SIZE_SCALE:.0f})")
        eng.tell(f"e match.getCharacters()[1].setX({there * SIZE_SCALE:.0f})")
        V.standing(eng, 2)
    eng.tick(20)
    # facing the other fighter on both engines: a fighter put down by its position keeps the way
    # it faced (its spawn's, which differs between the engines), so a short tap toward it turns it
    eng.tell("speed 1"); eng.speed = 1; eng.tell(f"seq {face}:4"); eng.tick(8); eng.faster(8)
    eng.tick(20)


# a direction pressed on the same frame as special reads as a flick on the source engine (samus's
# side special fires its super missile off it) and as a tilt here, so a side special is driven by
# the inputs the C6 gate uses on both engines: the direction held first, then special with it
INPUTS = {"special_side": "right:8 right+special", "special_side_air": "jump:3 right:8 right+special"}
if LEFT:
    INPUTS = {k: v.replace("right", "left") for k, v in INPUTS.items()}


def arm(eng, move):
    """a sequence plays one input per engine frame, so it runs at the engine's own rate; the
    speed is set before the recording opens, since a speed change reopens it"""
    if INPUTS.get(move) is not None:
        eng.tell("speed 1"); eng.speed = 1


def play(eng, move, is_ssf2):
    seq = INPUTS.get(move)
    if seq is None:
        eng.tell(f"move {move} p0")
    else:
        eng.tell(f"seq {seq}")
        eng.tick(sum(int(t.split(":")[1]) if ":" in t else 1 for t in seq.split()) + 2)
        eng.faster(8)


def ssf2_move(s, move):
    reset(s, True); arm(s, move)
    s.tell("record"); play(s, move, True); s.tick(FRAMES)
    mk = s.tell("trace raw"); s.reply(mk, "TRACERAW:", 20); s.tick(2)
    rows = []
    for l in s.log()[mk:].splitlines():
        m = re.match(r"(?:<< )?\s*\d+\t(.*)", l)
        if m:
            rows.append(m.group(1).split("|"))
    first = last = None; most = 0; far = end = 0.0; dy = None
    for i, f in enumerate(rows):
        if len(f) < 14:
            continue
        # the projectile count and the first one's place sit at 11-13 (a probe's values may follow)
        try:
            n = int(f[11])
        except ValueError:
            continue
        most = max(most, n)
        if n > 0 and f[12] != "-":
            if first is None:
                first = i
                # where it starts, above (-) or below the thrower's position, in the source's pixels
                dy = float(f[13]) - float(f[3])
            last = i
            d = abs(float(f[12]) - float(f[2]))
            far = max(far, d); end = d
    runs = V.trace_runs(s.read_trace(s.tell("trace")))
    return summary(first, last, most, far, end, plays_of(runs), dy)


# one eval, and short: a long `e` takes the engine down
# only the watched fighter's own projectiles count (both fighters are the same one, and the other's
# telly vision followed the other fighter)
# the fighter's animation transitions are kept too: a replay of the move by its own code (a catch)
# is a `playAnimation`, which this engine's animation feed does not log, so `plays` is read here
FM_WATCH = ("e@p0 (function(){var e=self.exports;e.__pp={f:0,a:-1,b:-1,m:0,r:0,z:0,y:0,t:[]};"
            "e.__pi=self.addTimer(1,-1,function(){var s=e.__pp;s.f++;var c=0;var w;"
            "for(q in match.getProjectiles())if(q.exports.__np!=true&&q.getOwner()==self){c++;w=q;}"
            "var n=self.getAnimation();if(n!=s.l){s.l=n;if(s.t.length<40)s.t.push(n);}"
            "if(c>s.m)s.m=c;if(c>0){if(s.a<0){s.a=s.f;s.q=self.getY();}if(s.f-s.a==2)s.y=w.getY()-s.q;"
            "s.b=s.f;var d=Math.abs(w.getX()-self.getX());if(d>s.r)s.r=d;s.z=d;}},{persistent:true});return 1;})()")
# the engine takes a command line of a fixed size, and one past it crashes the engine outright
# (OutsideBounds as it reads the line): a watcher that grows past it has to be cut, not sent
EVAL_LIMIT = 500
assert len(FM_WATCH) <= EVAL_LIMIT, f"FM_WATCH is {len(FM_WATCH)} characters, over the engine's {EVAL_LIMIT}"


def fm_move(f, move):
    reset(f, False)
    m = f.tell(FM_WATCH); f.reply(m, "E:", 10)
    arm(f, move)
    f.tell("record"); play(f, move, False); f.tick(FRAMES * 2)
    # read tagged: an untagged reply can be an earlier command's answer
    got = f.eval("(function(){var s=match.getCharacters()[0].exports.__pp;return [s.a,s.b,s.m,Math.round(s.r),Math.round(s.z),Math.round(s.y)];})()")
    trans = (f.eval("match.getCharacters()[0].exports.__pp.t.join(\" \")") or "").split()
    f.tell("e@p0 self.removeTimer(self.exports.__pi)")
    f.read_trace(f.tell("trace"))
    # a play is the move entered from outside it: the port cuts a move's clip into sections
    # (`special_up`, `special_up_fall`), and the move's own jump from a later section back to an
    # earlier one is the same play, as it is on the source's one label
    # (and the helpless fall the port gives a move's falling section, `fall_special`, which the
    # source plays under the move's own label: pac-man's trampoline bounces back into his up
    # special from it, one play on the source)
    family = lambda a: a == move or a.startswith(move + "_") or a == "fall_special"
    if os.environ.get("PROJ_DEBUG"):
        print(f"[{move}] transitions: {' '.join(trans)}", file=sys.stderr)
    plays = sum(1 for i, a in enumerate(trans) if a == move and (i == 0 or not family(trans[i - 1])))
    nums = [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", got or "")]
    if len(nums) < 5 or nums[0] < 0:
        return summary(None, None, int(nums[2]) if len(nums) > 2 else 0, 0, 0, plays)
    # the counter runs on this engine's frames: two to a source frame. the watcher ticks with the
    # thrower's timers, ahead of the projectiles' own and of the frame's movement, so what it reads
    # is how the frame before ended: a projectile is first seen in the frame its maker made it
    # (that frame is its first, as on the source's end-of-frame record), its height is read two
    # ticks on, once its own first timers have placed it (lucario's force palm follows its thrower
    # from a timer), over where its thrower stood as it showed (simon's bat ring, fired as he
    # rises, read a frame of his rise low against where he stood two ticks on), and the last frame
    # it was seen in is the one after its last
    # its height in this engine's units, which are the source's times the size scale (`__sp`)
    dy = nums[5] / SIZE_SCALE if len(nums) > 5 else None
    first, last = int(nums[0] // 2), int((nums[1] - 1) // 2)
    return summary(first, max(first, last), int(nums[2]), nums[3], nums[4], plays, dy)


def differs(a, b):
    if (a.get("most", 0) > 0) != (b.get("most", 0) > 0):
        return True
    if a.get("back") != b.get("back") or a.get("plays") != b.get("plays"):
        return True
    if a.get("dy") is not None and b.get("dy") is not None and abs(a["dy"] - b["dy"]) > 12:
        return True
    la, lb = a.get("life") or 0, b.get("life") or 0
    return lb > 0 and abs(la - lb) > max(6, lb // 4)


def main():
    subjects = [a for a in sys.argv[1:] if not a.startswith("--")]
    moves = opt("moves", ",".join(MOVES)).split(",")
    if not subjects:
        print(__doc__)
        return 2
    bad = total = 0
    base_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "proj_parity_baseline.json")
    try:
        baseline = set(json.load(open(base_path, encoding="utf8")).get("matching", []))
    except (OSError, ValueError):
        baseline = set()
    matching, regressed = set(), []
    for cid in subjects:
        if "--fresh" in sys.argv:
            fresh(cid)
        f = V.Engine(f"projparity-{cid}")
        f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
        T.entrance_over(f, ("ANIM:INTRO",))
        fm = {m: fm_move(f, m) for m in moves}
        f.close(final=True)
        s = V.Ssf2Engine(f"projparity-ssf2-{cid}")
        s.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_SSF2])
        s.faster(8); s.tick(200)
        src = {m: ssf2_move(s, m) for m in moves}
        s.close(final=True)
        # a move this engine never played while the source fired from it is played again in a
        # session of its own: one that stops answering partway through a run skips the rest
        # (lucario's side special and pikachu's specials, matching alone)
        again = [m for m in moves if fm[m].get("plays", 0) == 0 and src[m].get("most", 0) > 0]
        if again:
            f = V.Engine(f"projparity-{cid}-again")
            f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
            T.entrance_over(f, ("ANIM:INTRO",))
            for m in again:
                fm[m] = fm_move(f, m)
            f.close(final=True)
        for m in moves:
            a, b = fm[m], src[m]
            if a.get("most", 0) == 0 and b.get("most", 0) == 0:
                continue
            total += 1
            d = differs(a, b)
            bad += d
            key = f"{cid}/{m}" + ("<" if LEFT else "")
            if not d:
                matching.add(key)
            elif key in baseline:
                regressed.append(key)
            print(f"{'<<' if d else '  '} {cid:14} {m:16} fm   {a}\n{'':34}ssf2 {b}", flush=True)
    print(f"\n{total} move(s) with projectiles, {bad} differing")
    if "--save" in sys.argv:
        # the moves that match, kept with the ones this run did not measure
        keep = {k for k in baseline if k.split("/")[0] not in subjects or k.endswith("<") != LEFT}
        json.dump({"matching": sorted(keep | matching)}, open(base_path, "w", encoding="utf8"), indent=1)
        print(f"saved {len(keep | matching)} matching move(s) to {os.path.basename(base_path)}")
    if "--check" in sys.argv:
        if regressed:
            print(f"REGRESSED: {len(regressed)} move(s) the baseline lists as matching differ now: {', '.join(regressed)}")
            return 1
        print("no move the baseline lists as matching differs")
        return 0
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
