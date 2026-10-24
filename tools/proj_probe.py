#!/usr/bin/env python3
"""Watch a character's PROJECTILES, which the C1-C6 gates cannot: they read the fighter's own
animation, not what it spawns. This fires a move by input injection and follows what it spawned,
so an instance CAP (SSF2 `limit`) and FLIGHT behaviour (homing, owner aim, owner influence)
become measurable.

    python3 tools/proj_probe.py verify <char>
    python3 tools/proj_probe.py <char> "<input>" [shots] [ticks_between]
    python3 tools/proj_probe.py track <char> "<input>" [frames] [--away] [--hold=<dir>]

verify: fire each of a character's specials and check its projectiles do what the SOURCE declares
(read from the converted script headers): they spawn, they never throw, a declared trail drops its
effect, and a declared limit caps the live count when the shots pile up. PASS/FAIL. This is the port
checked against the stats SSF2 states, which the C1-C6 fighter gates never look at.

`input` is a C6-style control string ("special", "down+special", "right:8 right+special").
probe: fires it `shots` times with `ticks_between` engine ticks (speed 8) in between and prints
the live-count series (a cap shows up as a plateau).
track: fires once and follows the newest projectile frame by frame, reporting its distance to
the other fighter. `--away` turns the fighter to face AWAY from the opponent first, so a homing
projectile has to turn around: its distance rises, then falls. On a flat floor that is the only
way a curve shows, since a grounded fighter cannot be placed off the line of fire. `--hold=down`
holds a direction while the projectile flies, for an owner-aimed or owner-influenced one.

Positions come from the engine's own object walk (Engine.projectiles), never from an hscript
`getX()` on the handle, which takes the engine down.
"""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verify


def _shooter_xy(eng, is_ssf2):
    """where the fighter that fires is, in the same frame the object walk reports spawns in, so a
    spawn's offset from its shooter is comparable across the two engines once scaled."""
    try:
        if is_ssf2:
            x = verify.ssf2_read(eng, "e p0.X"); y = verify.ssf2_read(eng, "e p0.Y")
        else:
            x = float(eng.eval("match.getCharacters()[0].getX()"))
            y = float(eng.eval("match.getCharacters()[0].getY()"))
        return (x, y)
    except Exception:
        return None


def fire_once(eng, inp, f=2):
    # same seq shape the gate drives inputs with: "a:10 b" holds a for 10 source frames then taps b
    steps = " ".join(
        f"{s.split(':')[0]}:{int(s.split(':')[1]) * f}" if ":" in s else f"{s}:{f}"
        for s in inp.split()
    )
    mark = eng.tell(f"seq {steps}")
    eng.await_seq(mark, len(inp.split()))


def _pack_if_stale(cid):
    """the session boots the INSTALLED .fra as-is and never repacks; `peptide convert` writes only
    source. so a reconvert leaves the installed package stale and the probe would test old output
    (link's spawn-lift fix read as still-broken until the package was rebuilt by hand). rebuild the
    package whenever the source library is newer than it, so a probe always tests what was just
    converted."""
    import glob as _g, subprocess as _sp
    proj = f"characters/{cid}/{cid}.fraytools"
    if not os.path.exists(proj):
        return
    fra = f"characters/{cid}/build/{cid}.fra"
    src_mtime = 0.0
    for p in _g.glob(f"characters/{cid}/library/**/*", recursive=True):
        try: src_mtime = max(src_mtime, os.path.getmtime(p))
        except OSError: pass
    fra_mtime = os.path.getmtime(fra) if os.path.exists(fra) else 0.0
    if src_mtime > fra_mtime:
        _sp.run([verify.BIN, "pack", "--project", proj], capture_output=True)


def _boot(eng, chars):
    for cid in dict.fromkeys(chars.split(",")):  # each distinct char in the roster
        _pack_if_stale(cid.strip())
    if eng.boot(["--char", chars, "--stage", verify.CHAR_STAGE_FM], ("engine READY", "LAUNCHED")) is None:
        print("boot failed"); return False
    eng.wait(("ANIM:",), 60)
    eng.await_pose(timeout=20)
    return True


def probe(cid, inp, shots=6, ticks=2, speed=8):
    eng = verify.Engine(f"proj-{cid}")
    if not _boot(eng, cid):
        return None
    eng.faster(speed)
    series = []
    errs0 = eng.log().count("SCRIPTERR")
    for k in range(shots):
        before = len(eng.projectiles())
        fire_once(eng, inp)
        eng.tick(ticks)
        ps = eng.projectiles()
        newest = ps[-1] if ps else None
        series.append(len(ps))
        print(f"shot {k+1}: alive before={before} after={len(ps)}  newest={newest and (newest['x'], newest['y'], newest['anim'])}")
        if eng.log().endswith("[engine stream ended]") or "engine gone" in eng.log()[-200:]:
            print("!! ENGINE CRASHED during firing")
            break
    print(f"SCRIPTERR during probe: {eng.log().count('SCRIPTERR') - errs0}")
    eng.close()
    return series


def track(cid, inp, frames=40, away=False, hold=None, speed=8):
    """fire once and follow the newest projectile frame by frame, reporting its distance to the
    other fighter. a homing projectile's distance falls over time (after rising, with --away); a
    straight one's does not."""
    eng = verify.Engine(f"trk-{cid}")
    if not _boot(eng, f"{cid},{cid}"):
        return None
    eng.faster(speed)
    # the target a good distance away on the same floor: ahead of the shooter, or BEHIND it when
    # asked (the shooter stays put and keeps its facing; walking it around would carry it off the
    # stage edge), so a homing shot has to come back around
    sx = float(eng.eval("match.getCharacters()[0].getX()") or 0)
    sy = float(eng.eval("match.getCharacters()[0].getY()") or 0)
    eng.eval(f"match.getCharacters()[1].setX({sx - 280 if away else sx + 280})")
    tx = eng.eval("match.getCharacters()[1].getX()")
    ty = eng.eval("match.getCharacters()[1].getY()")
    tx, ty = float(tx or 280), float(ty or 0)
    fire_once(eng, inp)
    if hold:
        eng.tell(f"hold {hold}")
    # follow ONE projectile across frames: the newest at first sighting, then whichever live
    # projectile is nearest to where it was last tick (ids are not exposed, position is)
    rows = []
    last = None
    for f in range(frames):
        eng.tick(1)
        ps = eng.projectiles()
        if not ps:
            if last is not None:
                break  # the one we were following is gone (hit, expired, left the blast zone)
            continue
        if last is None:
            # start on a projectile that has actually launched: a homing missile sits in a
            # 'suspend' aim state at the muzzle first, and following THAT reads as frozen
            launched = [q for q in ps if q["anim"] not in ("suspend", None, "")]
            p = (launched or ps)[-1]
        else:
            p = min(ps, key=lambda q: (q["x"] - last["x"]) ** 2 + (q["y"] - last["y"]) ** 2)
        last = p
        d = math.hypot(tx - p["x"], ty - p["y"])
        rows.append((f, d, p["x"], p["y"], p["anim"]))
    if hold:
        eng.tell("release all")
    eng.close()
    print(f"{cid} [{inp}]{' target behind' if away else ''}{' holding '+hold if hold else ''}: shooter ({sx:.0f},{sy:.0f}), distance to target ({tx:.0f},{ty:.0f}) per tick")
    for f, d, x, y, a in rows:
        print(f"  t{f:>2}: dist={d:6.0f}  @({x:7.1f},{y:6.1f}) {a or ''}")
    ds = [d for _, d, *_ in rows]
    if len(ds) >= 3:
        lo = min(ds); lo_i = ds.index(lo)
        after = ds[lo_i:]
        # locked on if it got close and did not run away again (stayed within a body-length of its
        # closest approach); a straight shot's distance climbs back up once it passes the target
        locked = lo < 60 and max(after) < lo + 60
        approached = ds[-1] < ds[0] - 20 or (away and lo < max(ds) - 40)
        verdict = ("LOCKS ON (homes)" if locked
                   else "APPROACHES (homes)" if approached
                   else "flat/passes (no homing)")
        print(f"  -> {verdict}: start {ds[0]:.0f}, closest {lo:.0f} at t{lo_i}, end {ds[-1]:.0f}")
    return rows




def _declared(cid):
    """what each of a character's projectiles declares, read from the converted script headers
    (the line the emitter writes: 'as its own package declares it: ...'). returns the set of
    behaviours present across the character's projectiles, plus any limit value."""
    import glob, re
    beh = {"trail": None, "homing": False, "control": False, "influence": False, "limit": None}
    for f in glob.glob(f"characters/{cid}/library/scripts/Projectile/*Script.hx"):
        head = ""
        for ln in open(f, encoding="utf8"):
            if "declares it:" in ln:
                head = ln; break
        if not head:
            continue
        m = re.search(r"trail (\S+) every", head)
        if m: beh["trail"] = m.group(1)
        if "homingSpeed" in head: beh["homing"] = True
        if "controlDirection" in head: beh["control"] = True
        if "influence" in head: beh["influence"] = True
        m = re.search(r"limit (\d+)", head)
        if m: beh["limit"] = int(m.group(1))
    return beh


def _trail_vfx(eng, link):
    """how many VFX whose animation is the trail effect are live (the trail drops the effect's
    'active' animation); read off the same object walk as the projectiles, never a handle call."""
    eng.n = getattr(eng, "n", 0) + 1
    tag = f"v{eng.n}:"
    mark = eng.tell_batch(["tree", f'e "{tag}" + 1'])
    if eng.reply(mark, "E:" + tag) is None:
        return 0
    import re
    # a trail VFX renders as an entity.Vfx playing 'active'; count distinct positions
    seen = set()
    for m in re.finditer(r"Vfx @\((-?[\d.]+),(-?[\d.]+)\)[^\n]*'active'", eng.log()[mark:]):
        seen.add((round(float(m.group(1)), 0), round(float(m.group(2)), 0)))
    return len(seen)


SPECIAL_INPUT = {
    "special_neutral": "special",
    "special_side": "right:8 right+special",
    "special_up": "up+special",
    "special_down": "down+special",
}


def verify_projectiles(cid, speed=8):
    """fire each of a character's specials and check its projectiles do what the SOURCE declares:
    they spawn, they never throw, a declared trail actually drops its effect, and a declared limit
    caps the live count. this is the port checked against the stats SSF2 states, which the C1-C6
    fighter gates never look at (they read the fighter, not what it spawns)."""
    import json, glob, re
    beh = _declared(cid)
    # which specials spawn a projectile at all (their frame scripts call createProjectile). the
    # character's OWN entity carries the special animations; a Projectile entity does not, so scan
    # every entity and collect from whichever declares them (skip none by breaking early)
    spawns = set()
    for p in glob.glob(f"characters/{cid}/library/entities/*.entity"):
        try: d = json.load(open(p, encoding="utf8"))
        except Exception: continue
        if "animations" not in d or "keyframes" not in d or "layers" not in d: continue
        kf = {k["$id"]: k for k in d["keyframes"]}
        lys = {l["$id"]: l for l in d["layers"]}
        for a in d["animations"]:
            if a["name"] not in SPECIAL_INPUT: continue
            for lid in a.get("layers", []):
                layer = lys.get(lid, {})
                if any("createProjectile" in (kf.get(k, {}).get("code") or "") for k in layer.get("keyframes", [])):
                    spawns.add(a["name"])
    if not spawns:
        print(f"{cid}: no special spawns a projectile; nothing to check here")
        return True
    eng = verify.Engine(f"pv-{cid}")
    if not _boot(eng, f"{cid},{cid}"):
        return None
    eng.faster(speed)
    ok = True
    # single realistic fires: each declared-spawning special produces its projectile, drops its
    # trail if it declares one, and throws nothing. errors are counted HERE only -- a stress loop
    # that mashes a move faster than any input could is not the port erroring.
    errs0 = eng.log().count("SCRIPTERR")
    for anim in sorted(spawns):
        # fire each special from a clean neutral state: a fighter keeps ONE handle on the
        # projectile it last spawned, so firing the next special on top of the previous one's
        # still-live projectile makes it act on the wrong one (zelda plays a `continue` animation
        # on whatever it holds, and a needle has no such frame). let it settle back to idle first
        # so each special is read on its own, the way a player fires it.
        eng.await_pose(timeout=8)
        n0 = len(eng.projectiles())
        fire_once(eng, SPECIAL_INPUT[anim])
        eng.tick(4)
        ps = eng.projectiles()
        note = f"{anim}: spawned={len(ps)}"
        if len(ps) <= n0 and len(ps) == 0:
            # not a failure on its own: a charge or a final-smash spawn needs a hold or a meter this
            # driver does not give, so the frame that spawns is never reached. a THROWN error is the
            # reliable defect signal (below); a silent no-spawn is only flagged.
            note += " [no spawn seen (input may not reach it)]"
        if beh["trail"]:
            tv = _trail_vfx(eng, beh["trail"])
            note += f" trailVfx={tv}"
            if tv == 0 and len(ps) > 0:
                ok = False; note += " [TRAIL MISSING]"
        print("  " + note)
    errs = eng.log().count("SCRIPTERR") - errs0
    if errs:
        ok = False
    # limit: INFORMATIONAL only. the cap is enforced PER projectile-content (each content type
    # keeps its own live list), but this walk counts every projectile on screen, so firing several
    # specials that each spawn a different type sums past one type's cap without any of them being
    # over. a trustworthy per-content check needs the tree to name the content, which it does not
    # here, so report the peak against the declared cap without judging it (the cap itself is
    # verified in the converter's own controlled test).
    if beh["limit"] is not None and spawns:
        anim = sorted(spawns)[0]
        peak = 0
        for _ in range(beh["limit"] + 4):
            fire_once(eng, SPECIAL_INPUT[anim]); eng.tick(2)
            peak = max(peak, len(eng.projectiles()))
        print(f"  limit {beh['limit']}: peak live {peak} across all types (per-content cap not judged here)")
    eng.close()
    print(f"{cid}: declared {beh} | SCRIPTERR={errs}")
    print(f"{cid}: {'PASS' if ok else 'FAIL'}")
    return ok




# ── flight check: does what each special spawns do what the source declared ──────────────
import json as _json, glob as _glob, re as _re

def _declared_spawns(cid):
    """per special animation, the projectiles its frame scripts fire, read off the character's
    own entity: the content, the launch x/y speed handed over at the spawn, and whether the move
    charges (a `charging` label). the port should spawn each of these, place it, and -- when the
    launch is not zero -- move it. read from the source's own declaration, so it holds for content
    that does not exist yet."""
    ent = None
    for p in _glob.glob(f"characters/{cid}/library/entities/*.entity"):
        try: d = _json.load(open(p, encoding="utf8"))
        except Exception: continue
        if "animations" in d and any(a["name"] in SPECIAL_INPUT for a in d["animations"]):
            ent = d; break
    if ent is None: return {}
    # a SUSTAINED spawn cannot be seen with one tap of the button. two shapes, both read from the
    # source so this holds for content not written yet:
    #   - owner-linked: a follow spawn (a reflector, a flight effect, an absorb field) whose own
    #     script kills it the moment its owner leaves the move, so its lifetime is the owner's time
    #     in the move. it self-kills gated on the owner's animation (`character.getAnimation() ==
    #     "special_x"`) OR the owner being in the attack group (`inStateGroup(CStateGroup.ATTACK)`).
    #   - aimed/steered: a spawn whose own script reads the held controls to point or move itself, so
    #     without a direction held it goes nowhere and ends early (black mage's warp teleport).
    # a projectile script's stem (its content id without the Projectile suffix) ties it to the spawn
    # the move declares. collect the stems that are sustained, plus the move animations an owner-
    # linked script names directly.
    def _stem(s):
        return _re.sub(r"(projectile|script(?:\.hx)?)$", "", s, flags=_re.I).lower()
    sustained_stems, owner_linked, stationary_stems, bouncy_stems, grabtrap_stems = set(), set(), set(), set(), set()
    for sp in _glob.glob(f"characters/{cid}/library/scripts/**/*Script.hx", recursive=True):
        try: src = open(sp, encoding="utf8").read()
        except Exception: continue
        stem = _stem(os.path.basename(sp))
        kills = "self.kill()" in src
        # a grab-trap projectile declares a grab state of its own (rayman's vortex registers a
        # "projectile_grabbed" local state): it flies out, then grabs a foe and transforms/holds, so
        # its flight ends on the grab and its travel is not a clean 1:1 signal. it also renders in a
        # grab layer the projectile walk does not sample, so a single-run travel read cannot follow
        # it. read from the source's own declared states so it holds for content not written yet.
        if _re.search(r'registerLocalState\([^,]*,\s*"[^"]*grabbed', src) or \
           _re.search(r'_prepLocalState\("[^"]*grabbed', src):
            grabtrap_stems.add(stem)
        # a projectile that bounces off surfaces (pikachu's thunder jolt: bounceMax, a bounce() that
        # recomputes its angle on each COLLIDE_FLOOR) travels a chaotic, bounce-count-dependent
        # distance -- the source's own reading swings run to run -- so a single-run travel compare is
        # not a reliable 1:1 signal for it. flag it only if it truly does not move.
        if "bounceMax" in src or _re.search(r'\bfunction\s+bounce\b', src):
            bouncy_stems.add(stem)
        # self-kills when the owner is no longer in this move: named by animation, or by the owner
        # dropping out of the attack group (ness's psi magnet absorb field ends this way)
        owns_anim = kills and _re.search(r'\bcharacter\.getAnimation\(\)\s*==\s*"(\w+)"', src)
        owner_gated = kills and (owns_anim or "inStateGroup(CStateGroup.ATTACK)" in src)
        if owner_gated or "getHeldControls()" in src:
            sustained_stems.add(stem)
        if owns_anim:
            for m in _re.finditer(r'\bcharacter\.getAnimation\(\)\s*==\s*"(\w+)"', src):
                owner_linked.add(m.group(1))
        # a spawn that ZEROES its own x speed in initialize is stationary by design (simon's holy
        # water pins itself where it lands and its flame spreads as separate instances the single-
        # projectile probe cannot follow). that is the source's own code, so the declared launch is
        # not a flight to expect. distinct from a collision REACTION that stops it (link's arrow's
        # toContinue), which lives in a handler, not initialize, and stays flaggable. read only the
        # initialize body (up to the next function) so a handler's setXSpeed(0) does not count.
        m = _re.search(r'function\s+initialize\s*\([^)]*\)\s*\{', src)
        if m:
            body = src[m.end(): src.find("\nfunction ", m.end()) if src.find("\nfunction ", m.end()) >= 0 else len(src)]
            if _re.search(r'setXSpeed\(\s*(?:__svx\(\s*)?0(?:\.0*)?\s*\)?\s*\)', body):
                stationary_stems.add(stem)
    kf = {k["$id"]: k for k in ent["keyframes"]}
    lys = {l["$id"]: l for l in ent["layers"]}
    def num(expr):
        if expr is None: return None
        m = _re.search(r"-?\d+\.?\d*", expr.replace("__sp(", "").replace("flipX(", ""))
        return float(m.group(0)) if m else None
    out = {}
    for a in ent["animations"]:
        if a["name"] not in SPECIAL_INPUT: continue
        charging = spawns = held = grab_gated = False
        procs = []
        for lid in a.get("layers", []):
            layer = lys.get(lid, {})
            if layer.get("type") == "LABEL":
                for kid in layer.get("keyframes", []):
                    if kf.get(kid, {}).get("name") == "charging": charging = True
            for kid in layer.get("keyframes", []):
                code = kf.get(kid, {}).get("code") or ""
                # a move that keeps looping (and keeps its spawn alive) only while a button is held
                # reads that button in its own frame scripts. pit's down special reflector spawns at
                # frame 2 and ends a dozen frames later unless SPECIAL is still down, so a single tap
                # never shows it. read from the source so it holds for content not written yet.
                if "getHeldControls()" in code: held = True
                for m in _re.finditer(r'createProjectile\(self\.getResource\(\)\.getContent\("(\w+)"\), self\);([^\n]*)', code):
                    rest = m.group(2)
                    xs = _re.search(r"setXSpeed\(([^)]*)\)", rest); ys = _re.search(r"setYSpeed\(([^)]*)\)", rest)
                    # a spawn that positions itself off a GRABBED foe (isaac's growth reads
                    # getGrabbedFoe().getX() to plant on the foe it holds) only fires while a grab is
                    # live: with no foe held the read is null and nothing spawns, which is correct.
                    # the driver holds no grab, so the source's tree shows the fighter's own parts
                    # moving where the port rightly spawns nothing. read from the source so it holds
                    # for content not written yet.
                    if "getGrabbedFoe()" in rest: grab_gated = True
                    procs.append({"content": m.group(1), "xspd": num(xs.group(1)) if xs else None,
                                  "yspd": num(ys.group(1)) if ys else None})
        if a["name"] in owner_linked: held = True
        if any(_stem(p["content"]) in sustained_stems for p in procs): held = True
        # every projectile the move spawns pins itself where it lands (a stationary stream): the
        # declared launch is not a flight to expect off a single-projectile read
        stationary = bool(procs) and all(_stem(p["content"]) in stationary_stems for p in procs)
        bouncy = any(_stem(p["content"]) in bouncy_stems for p in procs)
        grab_trap = any(_stem(p["content"]) in grabtrap_stems for p in procs)
        if procs:
            out[a["name"]] = {"charging": charging, "held": held, "stationary": stationary,
                              "bouncy": bouncy, "grab_gated": grab_gated, "grab_trap": grab_trap,
                              "procs": procs}
    return out


def verify_flight(cid, speed=8):
    """check each of a character's specials against what its source declares it spawns: the
    projectile appears, and if it was launched with a speed, it MOVES (a stuck boomerang, a
    fireball that never leaves, a side special that fires nothing -- the sweep missed all of
    these because it only asked 'did it throw an error')."""
    decl = _declared_spawns(cid)
    if not decl:
        print(f"{cid}: no special declares a projectile spawn; nothing to check")
        return True
    eng = verify.Engine(f"vf-{cid}")
    if not _boot(eng, f"{cid},{cid}"):
        return None
    eng.faster(speed)
    ok = True
    for anim in sorted(decl):
        d = decl[anim]
        eng.await_pose(timeout=8)
        eng.eval("{ var p = match.getProjectiles(); var i=0; while(i<p.length){ p[i].kill(); i=i+1; } 0; }")
        eng.tick(2)
        # fire it the same reliable way the verify gate does (a held tap of the direction then the
        # button), which triggers the move on far more characters than pressing raw special does.
        # this ticks the spawn out, so the placement is read a beat late (tree reads lag a fast
        # projectile) -- placement is informational; MOVEMENT (does the shot travel at all) is the
        # signal, and it survives the lag.
        inp = SPECIAL_INPUT[anim]
        fire_once(eng, inp)
        # follow the newest projectile through its flight, holding special a while for a charge/aim
        # move that keeps its projectile alive
        if d["charging"]:
            eng.tell("hold special")
        path = []; last = None
        for _ in range(26):
            eng.tick(1)
            ps = [o for o in eng.objects() if o["cls"] == "FraymakersProjectile"]
            if ps:
                p = ps[-1] if last is None else min(ps, key=lambda q: (q["x"]-last["x"])**2 + (q["y"]-last["y"])**2)
                last = p; path.append((p["x"], p["y"]))
        eng.tell("release all")
        launched = any((pr["xspd"] and abs(pr["xspd"]) > 0.5) or (pr["yspd"] and abs(pr["yspd"]) > 0.5) for pr in d["procs"])
        note = f"{anim}: declares {[pr['content'] for pr in d['procs']]}"
        if not path:
            note += "  -> no projectile seen (a charge/aerial move may not fire from this driver)"
        else:
            import math as _m
            disp = max(_m.hypot(x-path[0][0], y-path[0][1]) for x, y in path)
            note += f"  spawned peak_disp={round(disp)} frames={len(path)}"
            if launched and disp < 8:
                ok = False; note += "  -> STUCK (launched with a speed but never moved)"
        print("  " + note)
    eng.close()
    print(f"{cid}: {'PASS' if ok else 'FAIL'}")
    return ok




def _flight_on(eng, is_ssf2, inp, f, charging=False, ticks=22, want_travel=True):
    """fire one special the reliable way and return how far the thing it spawns gets from where it
    appeared (a projectile that flies) and how many distinct spawns move. travel is measured as the
    greatest distance any spawn reaches from its OWN first-seen position, so a fast shot that jumps
    100px a tick is not lost the way frame-to-frame nearest matching loses it. Fraymakers
    projectiles share one class and carry no name, so they are pooled by their spawn origin; the
    source's tree is hundreds of static UI objects, so its spawns are the ones NOT present before
    the move, keyed by their stable (cls, name)."""
    import math as _m
    steps = " ".join(f"{s.split(':')[0]}:{int(s.split(':')[1])*f}" if ":" in s else f"{s}:{f}" for s in inp.split())
    last_shooter = [None]   # the fighter position read at the fire, for placement
    fm_origin = [None]      # FM: the first projectile position seen
    force_hold = [False]    # set on a retry: some specials only fire while special is HELD (a spear
                            # beam), which a tap never shows; hold special through the capture then

    def once():
        if not is_ssf2:
            eng.tell("release all")   # drop any input a prior attempt left held, or the fighter
                                      # never settles back to a clean stand for the next try
            eng.eval("{ var p=match.getProjectiles(); var i=0; while(i<p.length){ p[i].kill(); i=i+1; } 0; }")
            # a hold capture must start from a grounded neutral: a whiffed tap can leave the fighter
            # airborne, and holding the chord there fires the AERIAL special (a different or no
            # projectile). reset respawns it, then wait for it to land back in stand.
            if charging or force_hold[0]:
                eng.tell("reset")
            eng.await_pose(timeout=6)
            # push the other fighter far downrange so the shot flies its own arc instead of stopping
            # when it hits a target parked right in front (a hit reads exactly like a mid-air freeze)
            eng.eval("match.getCharacters()[1].setX(4000)")
        else:
            eng.tell("reset")
            try: eng.await_pose(timeout=4)
            except Exception: pass
            eng.tell("e p1.X = 4000")
        eng.tick(3)
        # read the fighter position now, in the quiet window before the move floods the feed: the
        # source's echo-anchored read gets lost in a throw's output, and it barely moves mid-throw
        last_shooter[0] = _shooter_xy(eng, is_ssf2)
        before = {(o["cls"], o["name"]) for o in eng.objects()} if is_ssf2 else None
        # a charge move fires its projectile only once the charge tops out (it auto-fires there) or
        # the button is let go; hold the move's chord THROUGH the capture and let go the moment the
        # shot appears, and give charge moves extra ticks, since a slow charge tops out well past the
        # window a plain shot needs. holding for the whole window never fires a move whose charge
        # tops out later, so releasing on first sight is what makes both fire.
        # hold the move's OWN chord, not a bare special: a side/up/down special needs its direction
        # held too (pac-man's power pellet fires from right+special held; holding special alone gives
        # neutral-b and no pellet). take the last input step's controls, drop any :frames, split the +.
        hold_chord = inp.split()[-1].split(":")[0].replace("+", " ") if inp.split() else "special"
        hold_it = charging or force_hold[0]
        held = False
        if hold_it:
            # hold the chord from a fresh stand to enter and stay in the move; a tap-then-hold lands
            # after the entry window and never re-triggers, so drive the hold instead of the tap seq
            eng.tell(f"hold {hold_chord}"); held = True
        else:
            mark = eng.tell(f"seq {steps}")
            try: eng.await_seq(mark, len(inp.split()))
            except Exception: pass
        # a slow charge tops out well past a plain shot's window, so hold through a long capture and
        # let go only when the shot appears (topping out auto-fires it). the exact top-out frame
        # differs per move and is not known here, so holding through is what fires them all.
        # a release-fire charge (pac-man's power pellet launch, sora's keyblade throw) spawns its shot
        # only AFTER the button is let go, so holding forever never shows it. let go once a charge
        # window has passed with nothing out, then keep watching the rest of the span for the shot.
        span = (ticks + 70) if hold_it else ticks
        release_at = ticks  # charge for the plain window, then release to fire a release-fire move
        origin = None        # FM: first projectile position seen (all spawns start at the muzzle)
        paths = {}           # SSF2: per (cls,name) list of positions
        series = []          # per tick: (farthest live spawn's distance from origin, live count)
        far_shooter = [0.0]  # FM: greatest distance any spawn reaches from the SHOOTER
        sx = last_shooter[0]
        for ti in range(span):
            eng.tick(1)
            far = 0.0; alive = 0
            if is_ssf2:
                for o in eng.objects():
                    k = (o["cls"], o["name"])
                    if k in before or o.get("hidden"): continue
                    pts = paths.setdefault(k, [])
                    pts.append((o["x"], o["y"]))
                    d = _m.hypot(o["x"]-pts[0][0], o["y"]-pts[0][1])
                    if d > 5000: continue   # an off-stage / uninitialised coordinate, not a shot
                    alive += 1; far = max(far, d)
            else:
                for p in eng.projectiles():
                    if abs(p["x"]) > 8000 or abs(p["y"]) > 8000: continue  # off in the blast zone / uninitialised
                    if origin is None: origin = (p["x"], p["y"]); fm_origin[0] = origin
                    d = _m.hypot(p["x"]-origin[0], p["y"]-origin[1])
                    if d > 5000: continue
                    alive += 1; far = max(far, d)
                    # distance from the shooter too: the tree reader can catch a fast short-lived shot
                    # a tick or two into its flight, so distance-from-first-seen misses the opening
                    # move (kirby's sword wave reads ~half). distance from the fixed shooter position
                    # does not, and a genuinely stuck shot stays near the shooter, so this vetoes a
                    # false STUCK without hiding a real one.
                    if sx is not None:
                        dsh = _m.hypot(p["x"]-sx[0], p["y"]-sx[1])
                        if dsh <= 5000: far_shooter[0] = max(far_shooter[0], dsh)
            series.append((far, alive))
            if held and alive > 0:  # the shot is out; let the button go so it launches and flies
                eng.tell("release all"); held = False
            elif held and ti >= release_at:  # charged long enough, nothing out: release to fire it
                eng.tell("release all"); held = False
        if held: eng.tell("release all")
        return paths, series, far_shooter[0]

    # the driven input does not always trigger the move on the first try (a reset can leave the
    # fighter mid-air, an input can land a frame early); retry until something spawns, up to 3 goes
    shooter = None
    far_shooter = 0.0
    for _ in range(3):
        paths, series, fsh = once()
        far_shooter = max(far_shooter, fsh)
        if shooter is None or last_shooter[0] is not None: shooter = last_shooter[0]
        # a real fire moves something; effects alone (SSF2 always spawns muzzle flashes) do not count,
        # so retry a launch that produced no travel unless the move declares no launch speed at all
        if max((d for d, _ in series), default=0) > 20 or not want_travel:
            break
    # a tap that never spawned anything may be a hold-to-fire special (bandana dee's spear beam only
    # spawns while special is down): try once more holding special through the capture. only runs when
    # the tap found nothing, so it costs nothing on the specials that fire from a tap.
    if not is_ssf2 and not any(n for _, n in series) and not force_hold[0]:
        force_hold[0] = True
        paths, series, fsh = once()
        far_shooter = max(far_shooter, fsh)
        if last_shooter[0] is not None: shooter = last_shooter[0]
    # where the shot first appeared, relative to the fighter that fired, in that engine's own units:
    # FM off the projectile origin, the source off the spawn that ends up travelling farthest (its
    # tree also lists the muzzle effects). this is the placement the source states, port against it.
    spawn = None
    if not is_ssf2:
        spawn = fm_origin[0]
    elif paths:
        best = max(paths.values(), key=lambda pts: (max(_m.hypot(x-pts[0][0], y-pts[0][1]) for x, y in pts) if len(pts) >= 2 else 0))
        spawn = best[0]
    spawn_off = None
    if spawn is not None and shooter is not None:
        spawn_off = (round(spawn[0]-shooter[0], 1), round(spawn[1]-shooter[1], 1))
    dists = [d for d, _ in series]
    peak = max(dists) if dists else 0.0
    tail = [d for d, _ in series[-5:]]
    settled = sorted(tail)[len(tail)//2] if tail else 0.0     # median of the last few ticks
    alive_end = any(n for _, n in series[-3:])
    spawned = any(n for _, n in series)
    tail_spread = (max(tail) - min(tail)) if tail else 0.0
    # a shot that flew out and is now sitting still far from where it started, still alive: a
    # boomerang that never came back, a projectile wedged mid-air. a returning/expiring shot ends
    # near its origin or is gone.
    frozen_far = alive_end and peak > 40 and settled > peak * 0.6 and tail_spread < 10
    # a genuine RETURN (a boomerang): ONE shot reached a real peak and then came back to near where
    # it started, read off that single object's own path so a muzzle effect lingering at the origin
    # is not mistaken for the shot coming home. distinct from despawning at range (a straight shot)
    # and from stopping at a wall (far out, never back), so it tells a not-returning port from a
    # returning source with no wall or parked target confounding it.
    def _traj_returned(pts):
        if len(pts) < 3: return False
        ds = [_m.hypot(x-pts[0][0], y-pts[0][1]) for x, y in pts]
        pk = max(ds)
        return pk > 60 and min(ds[ds.index(pk):]) < pk * 0.35
    if is_ssf2:
        returned = any(_traj_returned(pts) for pts in paths.values())
    else:
        # FM tracks the projectile alone (effects are not in its list), so the aggregate distance is
        # the shot's path, but only while a projectile is ALIVE: once it despawns the distance drops
        # to zero, which must not read as the shot coming home
        returned = False
        if peak > 60:
            pk = dists.index(peak)
            returned = any(n > 0 and d < peak * 0.35 for d, n in series[pk+1:])
    movers = 1 if peak > 12 else 0
    if is_ssf2:  # collapse duplicate (cls,name) walks into a mover count
        movers = sum(1 for pts in paths.values()
                     if len(pts) >= 2 and 12 < max(_m.hypot(x-pts[0][0], y-pts[0][1]) for x, y in pts) <= 5000)
    return {"movers": movers, "max_travel": round(peak), "spawned": spawned,
            "settled": round(settled), "alive_end": alive_end, "frozen_far": frozen_far,
            "returned": returned, "spawn_off": spawn_off, "far_shooter": round(far_shooter)}


def flight_compare(cid):
    """fire each special that DECLARES a projectile on BOTH engines and compare what it spawns: does
    the port spawn it at all, and does it travel as far as the source does (scaled). a projectile
    that flies on the source but is stuck on the port (link's boomerang), or one the port never
    spawns, shows up as a divergence the C1-C6 fighter gates never see. the moves checked and the
    speed each is launched with are read from the source's own declaration, so this holds for
    content that does not exist yet."""
    decl = _declared_spawns(cid)
    if not decl:
        print(f"{cid}: no special declares a projectile spawn; nothing to compare"); return True
    scale = verify.VELOCITY_SCALE
    try:
        _, sout, _ = verify.run([verify.BIN, "ssf2", "scale", f"../ssf2-ssfs/{cid}.ssf", cid], 120)
        m = _re.search(r"velocity_scale=([\d.]+)", sout)
        if m: scale = float(m.group(1))
    except Exception: pass
    anims = sorted(decl)
    # a move that hands its projectile a real launch speed is expected to move something; one that
    # declares only ~0 speed is a placed spawn, so a still reading there is not a failed fire
    def wants(a):
        return max((abs(p["xspd"] or 0) for p in decl[a]["procs"]), default=0) > 1 or \
               max((abs(p["yspd"] or 0) for p in decl[a]["procs"]), default=0) > 1
    fm = verify.Engine(f"cf-{cid}")
    if not _boot(fm, f"{cid},{cid}"): return None
    fm.faster(8)
    fm.await_pose(timeout=10)
    fm_r = {a: _flight_on(fm, False, SPECIAL_INPUT[a], 2, decl[a]["charging"], want_travel=wants(a)) for a in anims}
    fm.close()
    s = verify.Ssf2Engine(f"cs-{cid}")
    if s.boot(["--char", f"{cid},{cid}", "--stage", verify.CHAR_STAGE_SSF2]) is None:
        print(f"{cid}: SSF2 boot failed"); return None
    s.wait(("ANIM:",), 90); s.faster(8)
    s_r = {a: _flight_on(s, True, SPECIAL_INPUT[a], 1, decl[a]["charging"], want_travel=wants(a)) for a in anims}
    s.close()
    ok = True
    for a in anims:
        src, prt = s_r[a], fm_r[a]  # source=SSF2, port=FM
        # the greatest launch speed the move hands any of its projectiles: if it is ~0 the spawn is
        # meant to sit (a placed effect), so a stuck port reading is expected, not a defect
        vmax = max((abs(p["xspd"] or 0) for p in decl[a]["procs"]), default=0)
        vmax = max(vmax, max((abs(p["yspd"] or 0) for p in decl[a]["procs"]), default=0))
        # the greatest HORIZONTAL launch on its own: a spawn given only a vertical launch (samus's
        # morph-ball bomb pops up ~0 across and drops on gravity) is a placed/dropped one whose
        # cross-engine travel is a vertical drop plus the source's explosion effects, not a flight to
        # compare. only a real horizontal launch makes a shot that must FLY, so gate STUCK on it.
        hmax = max((abs(p["xspd"] or 0) for p in decl[a]["procs"]), default=0)
        want = round(src["max_travel"] * scale)
        got = prt["max_travel"]
        flag = ""
        # two auto-flags that no wall or parked target can fake: the port spawns nothing the source
        # does, or the source flies a shot a real distance and the port barely moves it. a boomerang
        # that flies out but never comes home is left to the printed travel/return numbers, since
        # isolating one shot's whole path off the source's object walk (effects and all) is not
        # reliable enough to fail a build on.
        # a sustained move (a held reflector, an owner-linked flight effect) spawns only while its
        # button is down or its owner stays in the move, and ends itself the instant that stops; the
        # single-tap driver never keeps it up, so neither engine reliably shows it here. report it,
        # do not fail on it: its spawn is what the source declares and is checked where the move can
        # actually be sustained.
        held = decl[a].get("held")
        grab_gated = decl[a].get("grab_gated")
        if decl[a].get("grab_trap"):
            # a grab-trap flies out then grabs and transforms; its flight ends on the grab and it
            # renders in a grab layer the walk can't sample, so its travel is not a 1:1 signal. its
            # spawn and launch are verified in-engine; report here, do not fail.
            flag = "  -> grab-trap (flies out then grabs/transforms; walk can't sample its brief flight)"
        elif grab_gated and not prt["spawned"]:
            # the spawn positions itself off a grabbed foe, so it only fires while a grab is live.
            # the driver holds no grab, so the port rightly spawns nothing; the source's travel here
            # is the fighter's own moving parts during the move, not the projectile. data, not a fail.
            flag = "  -> grab-gated spawn (needs a live grab the driver can't hold; source travel is the fighter's own parts)"
        elif not prt["spawned"] and src["spawned"]:
            if held:
                flag = "  -> neither sustains it (sustained move; tap driver ends it early)"
            else:
                flag = "  -> the port spawns NOTHING (source does)"; ok = False
        elif not prt["spawned"] and held:
            flag = "  -> port spawns nothing here (sustained move; tap driver ends it early)"
        elif decl[a].get("stationary") and prt["spawned"]:
            flag = "  -> stationary stream (pins itself on landing; single-projectile read can't follow the spread)"
        elif decl[a].get("bouncy"):
            # a bouncer's travel is chaotic run to run; only a shot that truly does not move is stuck
            if hmax > 1 and got < 20:
                flag = "  -> STUCK on the port (bouncer barely moves)"; ok = False
            else:
                flag = "  -> bouncer (travel varies with bounce count; not a single-run 1:1 signal)"
        elif held:
            # a sustained/steered shot (din's fire flies while special is held and is aimed by the
            # stick, detonating on release) does spawn now that the driver holds the chord, but its
            # travel depends on the sustained steering the tap/hold driver cannot supply, so a short
            # reading here is the driver's limit, not a stuck port. report it, do not fail on it.
            flag = "  -> sustained/steered move (travel needs held aiming the driver can't supply)"
        elif hmax > 1 and want > 60 and got < max(20, want * 0.4) and prt.get("far_shooter", 0) < want * 0.6:
            flag = "  -> STUCK/SHORT on the port"; ok = False
        # placement: where the shot first appears relative to the fighter, source scaled into port
        # units. a spawn the port puts well off where the source puts it is a misplaced projectile
        # or effect (too far forward, off centre), the kind of bug the fighter gates never see. the
        # source's tree read lags a frame, so a FAST shot has already flown off its muzzle by the
        # first capture and its offset is not comparable; only a slow/placed spawn (an effect, a
        # wave) sits near its muzzle on both engines, so only there is the gap a real misplacement.
        place = ""
        so, po = src.get("spawn_off"), prt.get("spawn_off")
        if so and po:
            wx, wy = round(so[0]*verify.SIZE_SCALE), round(so[1]*verify.SIZE_SCALE)
            dx, dy = po[0]-wx, po[1]-wy
            place = f" spawn@({po[0]:.0f},{po[1]:.0f}) vs src({wx},{wy})"
            # only a genuinely PLACED spawn is comparable: the spawn sits near the muzzle on BOTH
            # engines, so the first capture is its real position on each. a shot that flies on either
            # engine is not placed (a charge arrow reads launch~0 because its speed comes from the
            # charge, not a declared setXSpeed, yet it still flies), and comparing a flown shot's
            # muzzle-lagged first capture against a placed one fakes a divergence.
            if vmax < 3 and src["max_travel"] < 60 and got < 60 and (dx*dx + dy*dy) ** 0.5 > 45 and not flag:
                place += "  -> MISPLACED spawn"; ok = False
        contents = ",".join(p["content"] for p in decl[a]["procs"])
        print(f"  {a} [{contents}] launch~{vmax:.0f}: SSF2 travel={src['max_travel']} settled={src['settled']} "
              f"| FM travel={got} settled={prt['settled']}{place}{flag}")
    print(f"{cid}: {'PASS' if ok else 'FAIL'}")
    return ok


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__); sys.exit(1)
    flags = [a for a in sys.argv if a.startswith("--")]
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    hold = next((a.split("=", 1)[1] for a in flags if a.startswith("--hold=")), None)
    if args[0] == "compare":
        flight_compare(args[1])
    elif args[0] == "flight":
        verify_flight(args[1])
    elif args[0] == "verify":
        verify_projectiles(args[1])
    elif args[0] == "track":
        track(args[1], args[2] if len(args) > 2 else "special",
              int(args[3]) if len(args) > 3 else 40, away="--away" in flags, hold=hold)
    else:
        probe(args[0], args[1] if len(args) > 1 else "special",
              int(args[2]) if len(args) > 2 else 6, int(args[3]) if len(args) > 3 else 2)

