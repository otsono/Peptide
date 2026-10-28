#!/usr/bin/env python3
"""stage_cross_engine.py — run a stage in BOTH engines and compare what actually animates.

The static inventory proves the right objects were emitted; verify.py proves the
Fraymakers ones run. Neither answers the question that decides a port: does the converted
stage animate the SAME WAY the original does?

This runs the stage in SSF2 and in Fraymakers, reads each engine's own animation clock, and
compares the lengths. The conversion doubles every keyframe (SSF2 runs at 30fps, Fraymakers at
60), so the contract is checkable:

    fraymakers_total == 2 x ssf2_playable

where `ssf2_playable` is `totalFrames`, MINUS ONE when the clip's last frame carries a control
action. In SSF2 a frame's code runs BEFORE the frame renders, so a final frame holding
`gotoAndPlay` (or `endAttack`) jumps away without ever being shown — the clip reports N frames
and plays N-1. bowserscastle has both kinds: `bowsers_podoboos_bg` 82 -> 164 and
`bowsers_bubbles_bg` 40 -> 80 double cleanly, while `bowserSpectator` reports 142 and converts
to 282, which is 2x141, not 2x142. The live tree can't see whether that last frame carries
code, so BOTH are accepted and the report says which one matched.

A length outside both is a real defect with a visible symptom: an element whose loop is longer
or shorter than the original plays at the wrong speed or drifts out of phase with the stage.

Pairing is by ANIMATION LENGTH, not by name — the two engines name nothing alike (SSF2
`bowsers_podoboos_bg` vs `bowserscastlessf2_bowsers_podoboos_bg`), and the emitted ids carry a
stage prefix that isn't derivable in reverse. Length is the property under test, so any
element whose length pairs uniquely is checkable; ambiguous ones are reported, not guessed.

Usage:
  tools/tests/stage_cross_engine.py <stage>      # e.g. bowserscastle
"""
import json, os, re, subprocess, sys, time, collections

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "tools"))
import verify  # the engines: one session per game, driven through the same feed the gates use

# the fighter both engines are given: its own clips are not stage art and are left out
CHARACTER = "mario"


def engines(stage, fm_eng=None, ssf2_eng=None):
    """the two live engines on `stage`: the ones handed in (a match started in them), or
    booted here. either way the game is reused when one is already up."""
    if fm_eng is None:
        fm_eng = verify.Engine("sce-fm")
        if fm_eng.boot(["--char", CHARACTER, "--stage", f"{stage}ssf2"]) is None:
            raise SystemExit("the Fraymakers engine never launched the match")
    if ssf2_eng is None:
        ssf2_eng = verify.Ssf2Engine("sce-ssf2")
        if ssf2_eng.boot(["--char", f"{CHARACTER},{CHARACTER}", "--stage", stage]) is None:
            raise SystemExit("the SSF2 session never came up")
    return fm_eng, ssf2_eng


def await_walk(eng, marker, quiet=1.0, cap=420.0, since=0):
    """Wait for a tree walk to FINISH: its own first row (`marker`, after `since`), then no
    line at all for `quiet` seconds. A fixed wait is a guess, and a wrong guess truncates the
    walk mid-traversal. The rows arrive on the session's feed, so this blocks on them.
    SSF2 returns the whole tree in one write, so there is nothing until there is everything."""
    t0 = time.time()
    while marker not in eng.log()[since:]:
        if time.time() - t0 > cap:
            return False
        eng.next_line(1.0)
    while time.time() - t0 < cap:
        if eng.next_line(quiet) is None:
            return True
    return False


# Subtrees that are not stage BACKDROP ART, and so have no vfx counterpart by construction:
#   * the HUD/menu layer -- its own animating clips (scores, music info, sync banner)
#   * the fighters -- a character is its own port with its own timeline (mario reads as a
#     95-frame clip and would otherwise look like a missing 190-frame element)
#   * hazards -- these port to CustomGameObjects, not vfx, so they're a different comparison
# Hazards are detected from the CONVERTED SCRIPT rather than hardcoded: whatever the stage
# spawns via createCustomGameObject is a hazard on this stage.
NON_STAGE_ROOTS = ("menu_hud", "pause_menu", "help_menu", "training_menu",
                   # the KO overlay: parented under the stage, but match chrome rather than
                   # stage art, and it animates (65 frames) so it reads as a missing element
                   "screenkoholder")
CHARACTER_TOKEN = ["mario"]


def _frames_of(text):
    """{name: current frame} from an already-captured walk.

    Keyed by the RAW instance name, and looked up that way too — the first sample keys some
    elements by `parent > child`, and mixing the two schemes silently pairs unrelated objects,
    which is exactly what made an earlier version of the timing check produce nonsense.
    """
    out = {}
    for name, f in re.findall(r'"([^"]+)"\s*@\([^)]*\)[^\n]*?frame=(\d+)/\d+', text):
        out.setdefault(name, int(f))
    return out


def _labels_of(text):
    """{name: set of frame labels seen} from an already-captured walk.

    An SSF2 clip whose code jumps between frame labels is a state machine, not a loop: it only
    ever plays the labelled segments its script selects, never the whole timeline. Seeing more
    than one label on an element is how that shows up from outside, and it's the reason such an
    element's rate can't match a conversion that plays every frame end to end.
    """
    out = {}
    for name, lab in re.findall(
            r'"([^"]+)"\s*@\([^)]*\)[^\n]*?frame=\d+/\d+\s*\'([^\']+)\'', text):
        out.setdefault(name, set()).add(lab)
    return out


def hazard_and_character_names(stage, char):
    """Lowercase tokens whose SSF2 clips port to something other than a vfx."""
    sid = f"{stage}ssf2"
    path = os.path.join(ROOT, "stages", sid, "library", "scripts", "stage", f"{sid}Script.hx")
    # letters and digits only, both here and where these are matched against the source's clip
    # names: the port names its content without the underscores the source's clips carry
    # (`bombfactory_block` ships as `BombFactoryBlock`)
    norm = lambda t: re.sub(r"[^a-z0-9]", "", t.lower())
    out = {norm(char)}
    if os.path.exists(path):
        text = open(path, encoding="utf8").read()
        for ent in re.findall(r'createCustomGameObject\(\s*self\.getResource\(\)\.getContent\("([^"]+)"\)', text):
            out.add(norm(ent.replace(sid, "")))
        for ent in re.findall(r'createStructure\(\s*self\.getResource\(\)\.getContent\("([^"]+)"\)', text):
            out.add(norm(ent.replace(sid, "")))
    # ...and by the clips the plan says each object draws with: the port names an object by its
    # class (LunarCoreLaser) and the source's tree by its clip (lunarLaser), which share no token
    plan = plan_of(stage)
    for o in (plan.get("__objects") or []) + (plan.get("__spawned") or []):
        for c in o.get("clips") or []:
            out.add(norm(c))
    return {t for t in out if t}


def two_samples(eng, tree_cmd, marker, gap):
    """two tree walks exactly `gap` engine ticks apart: the second is sent once the engine
    has answered `tick gap`, which it does after the first walk was dispatched (a walk is
    one frame's work in the engine, its rows stream out after). taken at the engine's own
    rate. returns (walk1, walk2, gap)."""
    speed = getattr(eng, "speed", 1)
    if speed != 1:
        eng.faster(1)
    mark1 = eng.tell(tree_cmd)
    eng.tick(gap)
    mark2 = eng.tell(tree_cmd)
    quiet = 0.2 if marker == "[object " else 0.5
    await_walk(eng, marker, since=mark2, quiet=quiet)
    log = eng.log()
    text1, text2 = log[mark1:mark2], log[mark2:]
    # a walk that streams slowly (krazoa palace) lands some of its rows after the second walk
    # was sent. the engine answers in order, so the first walk ends where its tickables
    # section does: the marker's row states how many rows follow it
    if marker == "[tickables]":
        at = log.find(marker, mark1)
        if at >= mark2:
            while log.count(marker, mark1) < 2 and await_walk(eng, marker, since=log.rfind(marker) + 1, quiet=quiet):
                log = eng.log()
            n = re.match(r"\[tickables\] n=(\d+)", log[at:])
            end = at
            for _ in range(1 + (int(n.group(1)) if n else 0)):
                end = log.find("\n<<", end + 1)
                if end < 0:
                    end = len(log)
                    break
            text1, text2 = log[mark1:end], log[end:]
    if speed != 1:
        eng.faster(speed)
    return text1, text2, gap


def ssf2_animations(eng, stage, excluded, samples=4):
    """{name: total_frames} for every SSF2 STAGE object with a real timeline (total > 1).

    Walks with indentation so ancestry is known: a clip is stage content only if it is not
    inside one of the HUD/menu subtrees.
    """
    # Wait for a LIVE match, not for a guessed number of seconds. SSF2 decrypts its assets on
    # demand and the time varies; a walk taken too early returns the LOADING SCREEN (a progress
    # bar reads as `instance18 frame=76/100`), which looks like a stage with almost no objects
    # rather than like a mistimed read. The extra settle after that is for the hazards, which
    # the stage spawns once the match is running.
    if eng.wait(("ANIM:",), 120) is None:
        raise SystemExit("SSF2 never reached a match")
    # the hazards the stage spawns once the match is running: twenty-five seconds of the
    # source's frames, on its own clock
    eng.tick(750)
    if True:
        # depth 4: the per-torch clips live INSIDE `bowsers_torches_lit_bg`, and the
        # converter promotes each child to its own vfx, so a depth-3 walk stops one level
        # above the objects the Fraymakers side actually has.
        # two samples, with the tick gap measured across them
        text, text2, ticks = two_samples(eng, "tree 4", "[object ", 30)
        # Extra walks purely to observe LABELS. Two samples are enough to measure a rate but not
        # to enumerate the labels a state-machine element visits: catching it mid-`wait` rather
        # than mid-`idle` is chance, and one label looks like an ordinary loop. Each walk is
        # about a second, so a few more samples buy the distinction cheaply.
        # spaced by the engine's OWN frame counter (`await` runs out its budget against a parked
        # character's looping animation), because back-to-back walks span a couple of seconds and
        # an element can sit in one segment far longer than that.
        extra = ""
        for _ in range(samples):
            mark = eng.tell("await p0 150")
            await_walk(eng, "AWAIT:", since=mark, quiet=0.2)
            mark_tree = eng.tell("tree 4")
            await_walk(eng, "[object ", since=mark_tree, quiet=0.2)
            extra += eng.log()[mark:]
    out, hazards, skip_depth = {}, {}, None
    frames1 = {}
    raw_of = {}
    worlds = {}
    planes = {}
    class_of = {}   # key -> the source clip's CLASS (symbol), for matching plan __deferred entries
    # the clip classes the package declares (the plan's `symbols`): a class the package lacks is
    # the game's own, dropped into the stage by the match (an item's monitor landing in the
    # terrain plane) and not an element of the stage to pair
    declared = set(plan_of(stage).get("symbols") or [])
    # The SSF2 plane an element belongs to, read from its ancestry: these container clips are
    # the source's own depth model, and they're what the converter maps onto a Fraymakers
    # layer. Without this, "is it in the right layer" can only be answered on the FM side.
    PLANES = ("background", "midground", "foreground", "terrain")
    plane_at = {}
    # A deep walk is NOT atomic: it takes about a minute, and the live display list changes
    # under it, so index paths shift and the same object gets visited more than once
    # (`bowsers_podoboos_bg` appeared three times, at 50/82, 6/82 and 64/82). Identity is the
    # instance name plus its position, so a revisit collapses instead of inflating the count.
    seen_ids = set()
    parents = {}          # depth -> the nearest NAMED ancestor at that depth
    lengths_by_depth = {}  # depth -> the clip length recorded there (parent/child dedupe)
    # rows are "[idx]" THEN the depth indent, e.g. `[0]     [object menu_hud] "instance1878" …`
    row = re.compile(r'^(?:\[\d+\]\s?)?( *)\[object ([^\]]+)\]\s*"([^"]+)"\s*@\(([^)]*)\)'
                     r'\s*\S+\s*(?:HIDDEN\s*)?frame=(-?\d+)/(-?\d+)')
    # THE STAGE IS TWO SUBTREES OF THE ROOT: the stage clip (`stageMC`) and the container the
    # game hangs its camera backgrounds in. Everything else at the root is the game's own --
    # menus, the fighters' chrome, an item's tag -- and comes and goes on the game's clock, so a
    # walk that happens to meet one reads it as stage art missing from the port (mushroom
    # kingdom's `pid_mc`, a five-frame tag that appears with an item). Root children are kept by
    # what they hold, decided before the walk.
    rows = []
    for line in text.splitlines():
        line = line.replace("<< ", "")
        m = row.match(line)
        if m:
            rows.append((len(m.group(1)), m.group(2), m.group(3)))
    root_depth = min((d for d, _, _ in rows[1:]), default=0) if len(rows) > 1 else 0
    keep_roots = set()
    for i, (d, cls, name) in enumerate(rows):
        if d != root_depth:
            continue
        j = i + 1
        while j < len(rows) and rows[j][0] > d:
            j += 1
        sub = rows[i:j]
        if any(n == "stageMC" or c.startswith("stage_") for _, c, n in sub) \
                or any(re.search(r"_bg$", c, re.I) for dd, c, _ in sub if dd <= d + 2):
            keep_roots.add(i)
    row_i = -1
    for line in text.splitlines():
        line = line.replace("<< ", "")
        m = row.match(line)
        if not m:
            continue
        row_i += 1
        # a clip placed at no opacity is a placeholder the stage's code reads positions off, not
        # a thing anyone sees (bowser's castle marks its ember emitters with sixteen of them)
        if re.search(r" alpha=0\.00\b", line):
            continue
        indent, cls, name, pos, total = m.group(1), m.group(2), m.group(3), m.group(4), m.group(6)
        depth = len(indent)
        if depth == root_depth and row_i not in keep_roots:
            skip_depth = depth
            continue
        # a clip the source holds hidden is not a thing anyone sees, whatever its timeline does
        # (devil's machine keeps a 556-frame clip playing unseen); its subtree goes with it
        if re.search(r"\bHIDDEN\b", line):
            skip_depth = depth
            continue
        # an effect from the game's shared library (its classes carry the `global_` prefix) is
        # not the stage's: the port plays the engine's own shared effect in its place (the
        # thwomp's landing dust), which is not an element to pair
        if cls.startswith("global_"):
            skip_depth = depth
            continue
        if declared and cls not in declared and cls not in ("MovieClip", "Shape", "Sprite", "Bitmap", "StaticText", "TextField", "Main"):
            if os.environ.get("PEPTIDE_SCE_DUMP"):
                print(f"[ssf2-row] d={depth} {cls} {name!r} is the game's own (not a class the package declares)")
            skip_depth = depth
            continue
        # a fighter's pose clip carries its own name in the source and is a character's, not
        # the stage's, wherever the walk meets it
        if name == "stance":
            continue
        if os.environ.get("PEPTIDE_SCE_DUMP"):
            print(f"[ssf2-row] d={depth} {cls} {name!r} total={total}")
        # World position comes from the record's `abs=` — the node's world BOUNDING BOX
        # top-left. A node's own x/y is its registration point, which is not where the art
        # starts, and Fraymakers records a rasterised image's corner; comparing those two
        # produces a mapping that fits no element at all.
        ab = re.search(r"abs=\(([-\d.]+),([-\d.]+)\)", line)
        world = (float(ab.group(1)), float(ab.group(2))) if ab else None
        label = (re.search(r"'([^']+)'\s*$", line) or [None, None])[1] \
            if re.search(r"'([^']+)'\s*$", line) else None
        # remember a meaningful ancestor so an unnamed child can be reported by its parent
        # an unnamed clip stands for its own nearest named ancestor at its depth, so what sits under
        # it is not filed under a named sibling that came before it at that depth (jungle island's
        # three anonymous 140-frame clips beside `bg_bananas` held 5-frame clips the comparer
        # reported as the bananas')
        if not name.startswith("instance"):
            parents[depth] = name
        else:
            up = max((d for d in parents if d < depth), default=None)
            if up is not None:
                parents[depth] = parents[up]
            else:
                parents.pop(depth, None)
        for d in [d for d in plane_at if d >= depth]:
            plane_at.pop(d, None)
        if name.lower() in PLANES:
            plane_at[depth] = name.lower()
        cur_plane = plane_at.get(max(plane_at, default=-1)) if plane_at else None
        for d in [d for d in parents if d > depth]:
            parents.pop(d, None)
        if skip_depth is not None and depth > skip_depth:
            continue           # still inside a non-stage subtree
        skip_depth = None
        # the HUD/menu layer is identified by CLASS (its instances are named `instanceNNNN`)
        if cls in NON_STAGE_ROOTS or name in NON_STAGE_ROOTS:
            skip_depth = depth
        # an unnamed, classless, one-frame container outside every plane of the stage is the
        # engine's own (the layer its hit effects land in), not the stage's: what appears
        # under it appears when a fighter is struck, on either engine, at its own moment
        elif cls == "MovieClip" and name.startswith("instance") and str(total) == "1" and cur_plane is None:
            skip_depth = depth
            continue
        # a fighter or a hazard: ported, but not as a vfx. Hazards are still COMPARED —
        # against Fraymakers CustomGameObjects — just in their own section, because a hazard
        # is a state machine and its clock only means something next to the matching state.
        low = re.sub(r"[^a-z0-9]", "", cls.lower())
        hit = next((tok for tok in excluded if tok in low), None)
        if hit:
            # Recorded whatever the frame count: a hazard's SSF2 clip is very often ONE frame
            # driven entirely by AS3 (the lava is 1/1, the same pattern as EmberWeather), so a
            # `> 1` filter hides exactly the hazards worth looking at.
            if hit not in CHARACTER_TOKEN:
                hazards[cls if name.startswith("instance") else name] = (int(total), label)
            skip_depth = depth
            continue
        t = int(total)
        if t > 1:
            # A container and the clip that drives it report the SAME length -- they are one
            # animation, not two (`bowsers_podoboos_bg` and its inner clip are both 82 frames).
            # Compare against EVERY ancestor, not just the immediate parent: the driving clip
            # can sit more than one level down, and counting both inflates the SSF2 side into
            # a phantom "missing in Fraymakers".
            for d in [d for d in lengths_by_depth if d >= depth]:
                lengths_by_depth.pop(d, None)
            if t in lengths_by_depth.values():
                lengths_by_depth[depth] = t
                continue
            lengths_by_depth[depth] = t
            ident = (name, pos)
            if ident in seen_ids:
                continue
            seen_ids.add(ident)
            key = name
            if name.startswith("instance") and cls not in ("MovieClip", "Shape", "Sprite", "Bitmap"):
                # an anonymous instance of the clip's own class is that clip, by the name the
                # source gave the class (`clocktown_bottomfire_bg` placed with no instance name)
                key = cls
            elif name.startswith("instance"):
                # anonymous children of one clip are as many objects as they have lengths: a
                # tree of leaves rustling at eight frames and smoke at sixteen is not one child
                anc = max((d for d in parents if d < depth), default=None)
                key = f"{parents[anc]} \u25b8 child {t}f" if anc is not None else f"{cls} {t}f"
            out.setdefault(key, []).append(t)
            raw_of.setdefault(key, []).append(name)
            class_of.setdefault(key, cls)
            if world is not None:
                worlds.setdefault(key, world)
            if cur_plane:
                planes.setdefault(key, cur_plane)
            fr = re.search(r"frame=(\d+)/", line)
            if fr:
                frames1.setdefault(name, int(fr.group(1)))   # raw name: matches _frames_of
    labels = _labels_of(text)
    for sample in (text2, extra):
        for n, ls in _labels_of(sample).items():
            labels.setdefault(n, set()).update(ls)
    frames2 = _frames_of(text2)
    # a clip the source HOLDS: every instance under the key sits on the same frame in both
    # samples (a nested clip stopped on its first frame is a still, whatever its length says)
    # ...and in every later walk too: two samples a whole number of the clip's cycles apart see a
    # looping clip on the same frame (jungle island's sixteen-frame bananas), and the later walks
    # are spaced by another gap
    later = [_frames_of(w) for w in re.split(r"(?=>> await )", extra) if w.strip()]
    held = {k for k, names in raw_of.items()
            if out[k][0] > 1 and names and all(n in frames1 and n in frames2 and frames1[n] == frames2[n]
                                               and all(w.get(n, frames1[n]) == frames1[n] for w in later) for n in names)}
    # A name the source reuses for clips of DIFFERENT lengths (metalcavern has two clips both named
    # 'hueShift', one 1800 frames and one 960) is as many objects as it has distinct lengths.
    # Collapsing it to the first length and counting the rest as more of it demanded N port elements
    # of the wrong length and failed a stage that converts both correctly. Split it into one entry
    # per length so the doubling check pairs each to its own port element. Same-length repeats (16
    # torches all 8 frames) stay one entry. A split entry is left out of the position fit -- only
    # the first instance's world was kept under the shared name, so it has no coordinate of its own
    # to place, and a borrowed one would read as misplaced.
    ssf2_out, counts_out, worlds_out, planes_out, held_out, class_out = {}, {}, {}, {}, set(), {}
    for k, v in out.items():
        by_len = collections.Counter(v)
        if len(by_len) <= 1:
            ssf2_out[k] = v[0]
            counts_out[k] = len(v)
            if k in worlds: worlds_out[k] = worlds[k]
            if k in planes: planes_out[k] = planes[k]
            if k in class_of: class_out[k] = class_of[k]
            if k in held: held_out.add(k)
        else:
            for length, c in sorted(by_len.items(), key=lambda kv: -kv[0]):
                kk = f"{k} ▸ {length}f"
                ssf2_out[kk] = length
                counts_out[kk] = c
                if k in planes: planes_out[kk] = planes[k]
                if k in class_of: class_out[kk] = class_of[k]
                if k in held: held_out.add(kk)
    return (ssf2_out, counts_out, hazards, worlds_out, planes_out, frames1, frames2, ticks, labels, held_out, class_out)


def _clip_of_id(eid):
    """the source clip a made element's id carries: `<stage>ssf2_<clip>_<key>_<slot>`."""
    tail = eid.split("ssf2_", 1)[-1]
    parts = tail.split("_")
    return "_".join(parts[:-2]) if len(parts) > 2 else tail


def plan_of(stage):
    """the converter's own statement of what it made for this stage, when the package is
    still on disk: {(x, y): entry} for its vfx, each with the source clip it came from."""
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    import glob as _glob
    cands = [os.path.join(root, "stages", cid, "verify.json") for cid in (f"{stage}ssf2", stage)]
    cands += sorted(_glob.glob(os.path.join(root, "stages", f"{stage}*", "verify.json")))
    for p in cands:
        if os.path.exists(p):
            try:
                d = json.load(open(p, encoding="utf8"))
            except Exception:
                return {}
            out = {}
            # the clip classes the package declares, carried through for the row filter
            if d.get("symbols"):
                out["symbols"] = list(d["symbols"])
            for o in d.get("made", []):
                if o.get("kind") == "parallax" and "frames" in o:
                    # a camera background lives inside the stage entity, where no tree row shows
                    # it: counted from the plan, by length
                    out.setdefault("__parallax", []).append(int(o["frames"]))
                    continue
                if o.get("kind") == "deferred":
                    out.setdefault("__deferred", []).append(o)
                    continue
                if o.get("kind") == "on_demand":
                    # a clip the stage makes and removes at runtime (a random spawn stream): its
                    # head-count is RNG-driven, so it is checked for being made, not for a count
                    out.setdefault("__on_demand", set()).add(re.sub(r"[^a-z0-9]", "", str(o.get("clip", "")).lower()))
                    continue
                if o.get("kind") == "stage":
                    out["__stage"] = o
                    continue
                if o.get("kind") == "object" and "x" in o:
                    # a state machine: the sections it plays, by the clip it came from
                    out.setdefault("__objects", []).append(o)
                    continue
                if o.get("kind") == "spawned":
                    out.setdefault("__spawned", []).append(o)
                    continue
                if o.get("kind") != "vfx" or "x" not in o:
                    continue
                out[(round(float(o["x"]), 1), round(float(o["y"]), 1))] = o
            return out
    return {}


def transient_flash_lengths(stage):
    """Total animation lengths of converted VFX that are DRAWN on only a handful of frames -- a
    picture the source places for an instant inside a long loop. SSF2's tree is sampled at
    intervals, so a clip shown for one frame in hundreds is never enumerated on the source side;
    a faithful port of it therefore has no counterpart to pair against, not because the port
    invented it but because the source element is below the sampler's resolution. Such a length is
    reported as an unmeasurable transient, never failed. Read from the package the port emitted:
    a keyframe with no `symbol` draws nothing, so DRAWN frames are the ones that carry a symbol."""
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    import glob as _glob
    dirs = [os.path.join(root, "stages", f"{stage}ssf2"), os.path.join(root, "stages", stage)]
    dirs += sorted(_glob.glob(os.path.join(root, "stages", f"{stage}*")))
    out = set()
    for d in dirs:
        ents = _glob.glob(os.path.join(d, "library", "entities", "*.entity"))
        if not ents:
            continue
        for p in ents:
            try:
                e = json.load(open(p, encoding="utf8"))
            except Exception:
                continue
            kfs = e.get("keyframes", [])
            if not kfs:
                continue
            total = sum(int(k.get("length", 1)) for k in kfs)
            drawn = sum(int(k.get("length", 1)) for k in kfs if "symbol" in k)
            # a brief flash: drawn on a few frames of a long loop. post-double, <= 8 engine frames
            # is <= 4 source frames, well under the interval sampler's reach. a real backdrop
            # animation carries a symbol on ~every frame, so drawn ~= total.
            if total > 8 and drawn <= 8:
                out.add(total)
        break
    return out


# the sections the port's plane clips were seen in (read through the runtime, when it holds them)
PLANE_LABELS = {}


def fm_animations(eng, stage, samples=4):
    """{element: total_frames} for every converted vfx running in Fraymakers."""
    if eng.wait(("ANIM:STAND",), 120) is None:
        raise SystemExit("the Fraymakers match never reached STAND")
    # the same settle as the source's side, on this engine's clock
    eng.tick(240)
    if True:
        text, text2, fm_ticks = two_samples(eng, "tree", "[tickables]", 60)
        # extra samples for LABELS only, spaced by the engine's own frame counter — symmetric with
        # the SSF2 side. A segmented element is a custom game object here, and the question it has
        # to answer is which of its animations it actually plays, which two samples can't show.
        extra = ""
        for _ in range(samples):
            mark = eng.tell("await p0 150")
            # 150 frames at speed 8 is under two seconds; an await the engine never answers (a
            # fighter the stage keeps out of the poses awaited) must not cost the whole cap
            if not await_walk(eng, "AWAIT:", since=mark, quiet=0.5, cap=30.0):
                print(f"[{stage}] the engine did not answer an await; sampling on", file=sys.stderr)
            # the port's own plane clips, where its runtime holds them: which section each is in
            for pn in ("background", "foreground", "midground"):
                lab = eng.eval(f'Planes.get("{pn}").currentLabel')
                if lab and lab not in ("None", "null"):
                    PLANE_LABELS.setdefault(pn, set()).add(lab)
            mark_tree = eng.tell("tree")
            # an extra sample is for labels only: a walk that never reports its tickables (a
            # stage with thousands of them) is read as far as it got after two minutes
            if not await_walk(eng, "[tickables]", since=mark_tree, quiet=0.5, cap=120.0):
                print(f"[{stage}] a sample walk did not finish; read as far as it got", file=sys.stderr)
            extra += eng.log()[mark_tree:]
    # only the converted elements: the engine's own character vfx use other labels
    vfx = collections.Counter()
    vfx_pos = collections.defaultdict(list)
    pos_all = {}
    for x, y, _f, t in re.findall(
            r"Vfx @\(([-\d.]+),([-\d.]+)\) frame=(\d+)/(\d+) 'active'", text):
        vfx[int(t)] += 1
        vfx_pos[int(t)].append((float(x), float(y)))
        pos_all.setdefault((round(float(x), 1), round(float(y), 1)), int(t))
    # every vfx the walk saw, whatever it plays, for reading a verdict against what is there
    if os.environ.get("PEPTIDE_SCE_DUMP"):
        for x, y, f, t, lab in re.findall(
                r"Vfx @\(([-\d.]+),([-\d.]+)\) frame=(\d+)/(\d+) '([^']*)'", text):
            print(f"[fm-vfx] @({x},{y}) frame={f}/{t} '{lab}'", file=sys.stderr)
    # A vfx and its display container report the SAME local position, so the layer stamped on
    # the display row can be joined onto the entity that carries the clock. The two facts come
    # from different walks (tick stack vs display list) and position is what ties them.
    layer_at = {}
    for x, y, lay in re.findall(
            r"HeapsContainer @\(([-\d.]+),([-\d.]+)\)[^\n]*? layer=(\w+)", text):
        layer_at[(round(float(x), 1), round(float(y), 1))] = lay

    # hazards + moving structures, with the state they're currently in
    objs = [(cls, int(t), lab) for cls, t, lab in re.findall(
        r"pxf\.entity\.(CustomGameObject|AnimatedLineSegmentStructure) @\([^)]*\) "
        r"frame=\d+/(\d+) '([^']+)'", text)]
    # {total: current frame} at each sample, so an advance per tick can be measured
    # {total: {position: current frame}}: elements that share a length are told apart by where
    # they stand, so an advance is always one element's against itself (keyed by length alone,
    # the first of two same-length loops in each walk could be a different one, and read as a
    # loop running at the wrong rate)
    def clocks(t):
        out = {}
        for x, y, f, tot in re.findall(r"Vfx @\(([-\d.]+),([-\d.]+)\) frame=(\d+)/(\d+) 'active'", t):
            out.setdefault(int(tot), {}).setdefault((round(float(x), 1), round(float(y), 1)), int(f))
        return out
    # every animation each custom game object is seen playing, keyed by its position: a segmented
    # element ports to a CGO, so "which labels does it play" is the question that decides whether
    # the branch was reproduced, and one sample only ever shows one of them.
    cgo_anims = {}
    for sample in (text, text2, extra):
        for x, y, lab in re.findall(
                r"pxf\.entity\.CustomGameObject @\(([-\d.]+),([-\d.]+)\) frame=\d+/\d+ '([^']+)'",
                sample):
            cgo_anims.setdefault((round(float(x)), round(float(y))), set()).add(lab)
    # a converted state machine is a vfx playing the SEGMENT it is in, under that segment's
    # label; every label a vfx was seen under, so a branch the source takes is one the port took
    vfx_labels = {}
    for sample in (text, text2, extra):
        for x, y, lab in re.findall(r"Vfx @\(([-\d.]+),([-\d.]+)\) frame=\d+/\d+ '([^']*)'", sample):
            if lab and lab != "active":
                vfx_labels.setdefault((round(float(x), 1), round(float(y), 1)), set()).add(lab.lower())
    for k, v in vfx_labels.items():
        cgo_anims.setdefault((round(k[0]), round(k[1])), set()).update(v)
    return vfx, objs, vfx_pos, layer_at, clocks(text), clocks(text2), fm_ticks, cgo_anims, pos_all


def run(stage, fm_eng=None, ssf2_eng=None):
    """compare `stage` on both engines; the report goes to stdout. returns the list of
    differences (empty when both engines animate the same objects the same way)."""
    fm_eng, ssf2_eng = engines(stage, fm_eng, ssf2_eng)
    print(f"[{stage}] reading SSF2 animation clocks…", file=sys.stderr)
    excluded = hazard_and_character_names(stage, CHARACTER)
    print(f"[{stage}] not compared as backdrop art: {sorted(excluded)}", file=sys.stderr)
    # the extra walks exist to catch a state machine in each of its sections; a stage whose
    # plan has none needs only two samples of each element
    samples = 4 if (plan_of(stage) or {}).get("__objects") else 2
    (ssf2, ssf2_counts, ssf2_hazards, ssf2_world, ssf2_plane,
     ssf2_f1, ssf2_f2, ssf2_ticks, ssf2_labels, ssf2_held, ssf2_class) = ssf2_animations(ssf2_eng, stage, excluded, samples)
    print(f"[{stage}] reading Fraymakers animation clocks…", file=sys.stderr)
    fm, fm_objs, fm_pos, fm_layer, fm_c1, fm_c2, fm_ticks, fm_cgo_anims, fm_pos_all = fm_animations(fm_eng, stage, samples)

    ssf2_by_len = collections.Counter()
    for k, v in ssf2.items():
        ssf2_by_len[v] += ssf2_counts.get(k, 1)
    # An SSF2 plane maps onto exactly one Fraymakers depth container. Anything else is a
    # layering defect: the element exists and animates correctly but draws in front of, or
    # behind, things it shouldn't.
    PLANE_TO_LAYER = {
        "background": "BACKGROUND_BEHIND",
        "midground": "BACKGROUND_BEHIND",
        "foreground": "FOREGROUND_STRUCTURES",
    }

    # THE CONVERTER SAYS WHAT EACH ELEMENT CAME FROM. Its plan names the source clip behind every
    # vfx it made and where it put it, and the live tree reports each vfx at that same position,
    # so a Fraymakers element is paired to its SSF2 clip by NAME rather than guessed by length.
    # An element the plan marks as keeping the stage's whole timeline under a schedule (a clip
    # the stage puts down over and over) is the clip played at every moment the source plays it,
    # and its length is the stage's by design.
    plan = plan_of(stage)
    # elements the port draws for an instant (a picture flashed inside a long loop): the SSF2 tree
    # is sampled at intervals and never enumerates a one-frame clip, so these have no source
    # counterpart to pair by length -- an artifact of the sampler, not a port defect.
    transient = transient_flash_lengths(stage)
    if transient:
        print(f"transient flashes below the sampler's resolution (reported, not length-checked): "
              f"{sorted(transient)}")
    # clips the stage makes and removes at runtime (a random spawn stream): head-count is RNG-driven
    on_demand = plan.get("__on_demand", set()) if plan else set()
    by_clip = {}
    # pieces filed under a clip ABOVE their own: they serve that clip's timeline but are never
    # its leftovers (the bananas under the background plane are the bananas')
    borrowed = {}
    # what the port leaves out on purpose: an SSF2 clip of that class is not looked for
    deferred = plan.pop("__deferred", []) if plan else []
    # the stage's own painted art: the terrain plane's timeline may run several frames and draw
    # one picture, which the port paints once; the plan says how long the source's runs
    stage_art = plan.pop("__stage", None) if plan else None
    if stage_art and "terrain" in ssf2 and ssf2["terrain"] == stage_art.get("source_frames") \
            and stage_art.get("frames", 0) <= 2:
        print(f"terrain: the plane's {ssf2['terrain']}-frame timeline draws one picture here, painted once")
        ssf2.pop("terrain", None)
    # ...or moves nothing drawn over its frames (its markers and warning boxes walk, its art does
    # not): the picture it paints loops whatever clip loops inside, which is not the plane's length
    elif stage_art and "terrain" in ssf2 and ssf2["terrain"] == stage_art.get("source_frames") \
            and stage_art.get("plane_art_moves") is False:
        print(f"terrain: the plane's {ssf2['terrain']}-frame timeline moves nothing drawn; its picture loops the clips inside ({stage_art.get('frames')} frames here)")
        ssf2.pop("terrain", None)
    # ...and any other plane whose own timeline only places stills: the port read every frame of
    # it and found nothing that changes, and its children are compared on their own (crystal
    # smash's background runs 120 frames over two fixed plates)
    for plane_name in [n for n in list(ssf2) if n in PLANE_TO_LAYER and " \u25b8 " not in n]:
        pieces = [e for e in (plan or {}).values() if isinstance(e, dict) and e.get("clip") == plane_name]
        pieces += [e for e in (stage_art or {}).get("painted") or [] if e.get("clip") == plane_name]
        if pieces and all(int(e.get("frames") or 1) <= 2 for e in pieces):
            print(f"{plane_name}: the plane's {ssf2[plane_name]}-frame timeline places {len(pieces)} still(s) here, painted once")
            ssf2.pop(plane_name, None)
    # ...and the stage's root clip, whose timeline holds the planes: when the port cut no element
    # out of it and painted the stage as one picture, its frames move nothing the port keeps apart
    # (sky pillar's root runs 90 frames over fixed planes)
    if "stageMC" in ssf2 and stage_art and int(stage_art.get("frames") or 1) <= 2 \
            and not any(isinstance(e, dict) and e.get("clip") == "stageMC" for e in (plan or {}).values()):
        print(f"stageMC: the root's {ssf2['stageMC']}-frame timeline holds the planes over one painted picture")
        ssf2.pop("stageMC", None)
    left_out = {d.get("clip") for d in deferred if d.get("clip")} | {d.get("class") for d in deferred}
    # ...and by CLASS, normalized: a deferred entry names the source clip by its symbol
    # ('<stage>_fla.goalLightAnim_4'), while the live tree keys the clip by its instance name
    # ('goalAnim'). Reducing both to the symbol's last segment, letters and digits only, lets a
    # clip whose animation the port deferred (all its segments event-driven) be left out by name
    # OR by class, whichever the two sides share.
    norm_cls = lambda s: re.sub(r"[^a-z0-9]", "", str(s).rsplit(".", 1)[-1].lower())
    left_out_cls = {norm_cls(d.get("clip")) for d in deferred if d.get("clip")} | \
                   {norm_cls(d.get("class")) for d in deferred if d.get("class")}
    left_out_cls.discard("")
    for d in deferred:
        print(f"left out by the port: {d.get('class')} ({d.get('base')}), the clip {d.get('clip')!r}")
    for nm in list(ssf2):
        if nm in left_out or nm.split(" \u25b8 ")[0] in left_out \
                or norm_cls(ssf2_class.get(nm, "")) in left_out_cls:
            ssf2.pop(nm, None)
    for n in (plan.pop("__parallax", []) if plan else []):
        fm[n] += 1
    # the sections each source clip's ported objects play, and which they were seen in
    key_of = lambda s: re.sub(r"[^A-Za-z0-9]", "", s).lower()
    machines = {}
    for o in (plan.pop("__objects", []) if plan else []):
        clip = o.get("clip") or _clip_of_id(o.get("id", ""))
        pos = (round(float(o["x"])), round(float(o["y"])))
        seen = fm_cgo_anims.get(pos, set())
        m = machines.setdefault(clip, {"phases": set(), "seen": set(), "lens": set()})
        m["phases"].update(key_of(p) for p in o.get("phases", []))
        m["lens"].update(o.get("phase_lens", []))
        m["seen"].update(key_of(s) for s in seen)
    if plan:
        for (x, y), n in fm_pos_all.items():
            e = plan.get((x, y))
            if e is None:
                continue
            by_clip.setdefault(e.get("clip") or _clip_of_id(e.get("id", "")), []).append(
                (n, bool(e.get("scheduled")) or bool(e.get("moving")), e.get("id", "")))
            # ...and under every clip over it: a plane's own timeline is carried by the pieces cut
            # from it (hyrule temple's clouds run its foreground's 1601 frames), which the plan
            # names by the clips above them
            for over in e.get("names") or []:
                if over != (e.get("clip") or ""):
                    by_clip.setdefault(over, []).append((n, bool(e.get("scheduled")) or bool(e.get("moving")), e.get("id", "")))
                    borrowed.setdefault(over, set()).add(e.get("id", ""))
    print(f"\n{'ssf2 object(s)':40}{'ssf2':>6}{'x2':>6}{'fm':>4}  {'plane -> layer':38} verdict")
    print("-" * 96)
    bad = matched = 0
    seen_lengths = set()
    claimed_fm = {}
    # elements whose whole timeline came through at length: the labels seen on them across the
    # samples are marks on that timeline, not branches the source takes
    linear = set()
    if by_clip or machines:
        for nm in sorted(ssf2):
            length, count = ssf2[nm], ssf2_counts.get(nm, 1)
            base = nm.split(" \u25b8 ")[0]
            mine = by_clip.get(base) or by_clip.get(nm)
            m = machines.get(base) or machines.get(nm)
            if mine is None and m is not None:
                # ported as objects that play the clip's sections: the length contract is per
                # section, and the timing table reads which sections the objects were seen in
                seen_lengths.add(length)
                matched += 1
                plane = ssf2_plane.get(nm) or ssf2_plane.get(base)
                print(f"{nm[:39]:40}{length:>6}{'-':>6}{'-':>4}  {(plane or '?'):38} "
                      f"ok — a state machine over {len(m['phases'])} sections")
                continue
            if nm in ssf2_held:
                # held on one frame in the source across the samples: a still, which the port
                # paints; no loop length is owed for it
                seen_lengths.add(length)
                matched += 1
                plane = ssf2_plane.get(nm) or ssf2_plane.get(base)
                print(f"{nm[:39]:40}{length:>6}{'-':>6}{'-':>4}  {(plane or '?'):38} ok — held on one frame in the source, a still here")
                continue
            if mine is None:
                continue
            seen_lengths.add(length)
            want, want_ctrl = length * 2, (length - 1) * 2
            # the elements of this clip not yet claimed by a sibling of another length
            used = claimed_fm.setdefault(base, set())
            free = [(f, s, i) for f, s, i in mine if i not in used]
            # an element cut to a loop that divides the clip's length draws the same pictures
            # in the same order forever: a six-frame clip that blinks two pictures is that
            # an exact fit for a SIBLING is that sibling's: a parent seventeen frames long must
            # not take a child's sixty-eight-frame element as four of its own loops, or the child
            # finds nothing left and reads as a mismatch it is not (kingdom2's backdrop)
            siblings = {n for n in ssf2 if n != nm and n.split(" \u25b8 ")[0] == base}
            theirs = {v for n in siblings for v in (ssf2[n] * 2, (ssf2[n] - 1) * 2)} - {want, want_ctrl}
            def same_pictures(f):
                # ...or a longer loop the clip's whole length fits into a whole number of times: a
                # leaf that flutters over four frames while the timeline drifts it over sixteen
                if f in theirs: return False
                return f in (want, want_ctrl) or (0 < f < want and want % f == 0) or (f > want and f % want == 0)
            fits = [f for f, s, i in free if same_pictures(f)]
            # a child drawn inside a piece its parent already claimed, at a length the child's
            # loop fits a whole number of times, is drawn right (the ladybug's eight frames
            # inside its ninety-frame parent cut to their common period). the parent's own length
            # is exactly what CONTAINS the child, so the sibling exclusion (`theirs`, which keeps a
            # parent from stealing a child's separate element) must NOT block a child from being
            # recognised as embedded in that parent: check the whole-multiple fit directly here
            # (frosslass's 10f child loops 210 times inside its 2100f parent -> 4200 % 20 == 0).
            if not fits and " \u25b8 child" in nm:
                fits = [f for f, s, i in mine if i in used and f >= want and want > 0 and f % want == 0]
            # a clip whose only port piece is a still: the walk sampled every frame of the source's
            # timeline and saw one picture (gangplank galleon's hull runs nine frames of the same
            # hull), so the length owed is none
            still_only = len(mine) == 1 and mine[0][0] <= 4 and not mine[0][1]
            if not fits and still_only:
                fits = [mine[0][0]]
                used.add(mine[0][2])
            for f, s, i in free:
                if same_pictures(f):
                    used.add(i)
            sched = [f for f, s, i in free if s and not same_pictures(f)]
            # what is left over is only a mismatch once every sibling has had its pick
            last_of_clip = nm == max(n for n in ssf2 if n.split(" \u25b8 ")[0] == base)
            # ...and a leftover of four frames or fewer is a still: a picture the port keeps as its own
            # element (a target test's markers) that the source lists as no child at all
            other = [f for f, s, i in mine if not s and f > 4 and not same_pictures(f) and i not in used and i not in borrowed.get(base, set()) and f not in transient] if last_of_clip else []
            plane = ssf2_plane.get(nm) or ssf2_plane.get(base)
            if len(fits) >= count and not other:
                verdict, matched = f"ok ({len(fits)} by name)", matched + 1
                linear.add(nm)
            elif fits and not other:
                verdict = f"ok ({len(fits)} of {count} by name; {len(sched)} keep the stage's timeline under a schedule)"
                matched += 1
            elif not other and sched:
                verdict = f"ok ({len(sched)} keep the stage's timeline: placed under a schedule or moved by it, the clip inside at every moment the source draws it)"
                matched += 1
            elif not other and " \u25b8 child" in nm and m is not None and m.get("lens", set()) & {want, want_ctrl}:
                # a child that runs its own clock inside a clip that holds sections plays the
                # clip's sections, and its whole loop in each one the clip holds on
                verdict = f"ok — plays its own {want}-frame loop in the sections its owner holds"
                matched += 1
            else:
                verdict = f"LENGTH MISMATCH — by name: Fraymakers plays {sorted(set(other))} frames, expected {want} or {want_ctrl}"
                bad += 1
            print(f"{nm[:39]:40}{length:>6}{want:>6}{len(mine):>4}  {(plane or '?'):38} {verdict}")
    for length, count in sorted(ssf2_by_len.items(), key=lambda kv: -kv[0]):
        if length in seen_lengths and (by_clip or machines):
            continue
        names = sorted(n for n, v in ssf2.items() if v == length)
        if not names:
            continue   # every clip of this length was left out on purpose
        # accept 2xN and 2x(N-1): the latter is a clip whose last frame carries a control
        # action, which SSF2 runs but never renders (see the module docstring)
        want, want_ctrl = length * 2, (length - 1) * 2
        got, got_ctrl = fm.get(want, 0), fm.get(want_ctrl, 0)
        label = names[0] if len(names) == 1 else f"{names[0]} +{len(names)-1} more"
        # A segmented clip is converted DELIBERATELY short: only the segments its frame scripts can
        # reach are ported, so its Fraymakers length is a subset of the timeline and the doubling
        # contract does not apply. Checking it against 2x the whole timeline would demand back the
        # very frames the source never plays.
        seg_labels = sorted(set().union(*(ssf2_labels.get(n.split(" ▸ ")[0], set()) for n in names)))
        if len(seg_labels) > 1:
            print(f"{label[:39]:40}{length:>6}{'-':>6}{'-':>4}  "
                  f"{(ssf2_plane.get(names[0]) or '?'):38} "
                  f"segmented into {', '.join(repr(l) for l in seg_labels)} — "
                  f"plays a subset, not the {length}f timeline")
            continue
        made_on_demand = any(re.sub(r"[^a-z0-9]", "", n.split(" ▸ ")[0].lower()) in on_demand for n in names)
        if made_on_demand and (got or got_ctrl):
            # a stream the stage spawns and drops at runtime: the port makes it, and its head-count
            # is RNG-driven, so two independent populations never hold the same count at one frame
            verdict, matched = f"ok (on-demand: {max(got, got_ctrl)} alive, count is RNG-driven not fixed)", matched + 1
            shown = want if got else want_ctrl
        elif got >= count:
            verdict, matched = "ok", matched + 1
            shown = want
        elif got_ctrl >= count:
            verdict = "ok (last frame is a control frame: plays N-1)"
            matched += 1
            shown = want_ctrl
        elif got or got_ctrl:
            verdict = f"COUNT {max(got, got_ctrl)} in FM vs {count} in SSF2"
            shown = want
            bad += 1
        else:
            near = sorted(fm, key=lambda t: abs(t - want))
            verdict = (f"LENGTH MISMATCH — FM has {near[0]}, expected {want} or {want_ctrl}"
                       if near and abs(near[0] - want) <= 8
                       else f"MISSING — no Fraymakers element with {want} or {want_ctrl} frames")
            shown = want
            bad += 1
        # layer: SSF2's plane against the container the Fraymakers element landed in
        plane = next((ssf2_plane.get(n) for n in names if ssf2_plane.get(n)), None)
        fm_lay = None
        for p_ in fm_pos.get(shown, []):
            fm_lay = fm_layer.get((round(p_[0], 1), round(p_[1], 1)))
            if fm_lay:
                break
        want_lay = PLANE_TO_LAYER.get(plane or "")
        pair = f"{plane or '?'} -> {fm_lay or '?'}"
        if plane and fm_lay and want_lay and fm_lay != want_lay:
            pair += f" WANT {want_lay}"
            verdict = (verdict + "; " if verdict != "ok" else "") + "LAYER MISMATCH"
            bad += 1
        print(f"{label[:39]:40}{length:>6}{shown:>6}{max(got, got_ctrl):>4}  {pair[:37]:38} {verdict}")

    # count both accepted lengths per source clip (2N and, for a control-frame clip, 2(N-1))
    paired = 0
    for length in ssf2_by_len:
        paired += fm.get(length * 2, 0) or fm.get((length - 1) * 2, 0)
    # a segmented element's counterpart is deliberately a different length, so nothing pairs with
    # it by length — that is the design, not an orphan
    segmented = sum(1 for n in ssf2 if len(ssf2_labels.get(n.split(" ▸ ")[0], set())) > 1)
    # transient flashes are unmeasurable on the source side, so they are not orphans either
    transient_count = sum(c for l, c in fm.items() if l in transient)
    unpaired = max(0, sum(fm.values()) - paired - segmented - transient_count)
    print("-" * 96)
    if unpaired:
        print(f"note: {unpaired} Fraymakers element(s) had no SSF2 counterpart by length")
    print(f"{matched}/{len(ssf2_by_len)} animation lengths match the 30->60fps doubling contract\n")

    # ── timing ──────────────────────────────────────────────────────────────────
    # Matching LENGTHS says an element loops over the right number of frames; it says nothing
    # about the RATE. An element that steps every other tick has the correct length and plays
    # at half speed, and every other check here would pass it.
    #
    # No tick counter is needed, which matters because Fraymakers' per-frame recorder feeds an
    # in-process buffer rather than the session log. Within ONE engine every element advances
    # once per tick, so between two samples they must all advance by the SAME amount — an
    # element that doesn't is mistimed relative to its own stage. Comparing each element to its
    # engine's median makes the check self-referential and immune to how fast the host happened
    # to be. Across engines the two medians should differ by ~2x, which is the 30->60 step.
    def advance(f1, f2, total):
        if f1 is None or f2 is None or not total or total <= 1:
            return None
        return (f2 - f1) % total

    # the two samples are a known number of the engine's own ticks apart (`ssf2_ticks`,
    # `fm_ticks`, the second twice the first), so a clip that steps once a tick has advanced
    # exactly that, modulo its length. one that has not is driven by its own code (a state
    # machine, a scripted hop) and is reported as such; the defect is an element that steps
    # with the clock on one engine and not on the other
    # either walk can be taken a few frames after it was asked for (the first lands late when the
    # engine is busy as it is sent, the second as the engine answers the tick), so a clock loop
    # lands within a few frames of the gap either way; a loop at the wrong rate is off by a share
    # of the whole gap (half a sixty-frame gap at half rate), far outside it
    def with_clock(adv, ticks, total):
        return adv is not None and total and (adv - ticks) % total in set(range(0, 7)) | set(range(total - 6, total))
    # the advance of the Fraymakers loops of one length, each measured against itself: the one
    # that steps with the clock where any does (the pairing is by length, so any of them is the
    # counterpart), else the first
    def fm_advances(total):
        a, b = fm_c1.get(total) or {}, fm_c2.get(total) or {}
        got = [advance(a[k], b[k], total) for k in a if k in b]
        return [g for g in got if g is not None]
    def fm_advance(total):
        got = fm_advances(total)
        if not got:
            return None
        on = [g for g in got if with_clock(g, fm_ticks, total)]
        return on[0] if on else got[0]

    rows = []
    for length in sorted(ssf2_by_len, reverse=True):
        names = sorted(n for n, v in ssf2.items() if v == length)
        # the reported key may be `parent > child`; the frame samples are keyed by raw name
        if not names:
            continue   # every clip of this length was left out on purpose
        nm = names[0]
        raw = nm.split(" \u25b8 ")[0] if " \u25b8 " in nm else nm
        src = raw if raw in ssf2_f1 else nm
        want = length * 2 if (length * 2) in fm_c1 else (length - 1) * 2
        labs = sorted(ssf2_labels.get(src, ()))
        # a clip the port made a state machine is one, whether or not the source walk could
        # read its labels off the clip (a plane the stage's class drives keeps them on its class)
        m = machines.get(raw) or machines.get(nm)
        if len(labs) <= 1 and m and len(m.get("phases", ())) > 1:
            labs = sorted(m["phases"])
        # the source's clips of this length each measured against themselves, the one that steps
        # with the clock where any does, as the port's side is chosen (a name sorting first that
        # its own code drives read as a half-rate pair and passed on a rerun)
        def src_of(n):
            r = n.split(" \u25b8 ")[0] if " \u25b8 " in n else n
            return r if r in ssf2_f1 else n
        got = [(advance(ssf2_f1.get(src_of(n)), ssf2_f2.get(src_of(n)), length), n) for n in (names if len(labs) <= 1 else [nm])]
        got = [g for g in got if g[0] is not None]
        # the pairing is by length, so any clip of it on one side is the counterpart of any on the
        # other: a pair that behaves alike on both (clock or code) is taken where there is one
        f_all = fm_advances(want)
        f_clock = [g for g in f_all if with_clock(g, fm_ticks, want)]
        f_code = [g for g in f_all if not with_clock(g, fm_ticks, want)]
        pick = None
        for g in got:
            if with_clock(g[0], ssf2_ticks, length) and f_clock: pick = (g, f_clock[0]); break
            if not with_clock(g[0], ssf2_ticks, length) and f_code: pick = (g, f_code[0]); break
        if pick is None:
            pick = (got[0] if got else (None, nm), fm_advance(want))
        (sa, nm), fa = pick
        rows.append((nm, length, sa, fa, labs))

    seg = lambda r: len(r[4]) > 1
    s_med, f_med = ssf2_ticks, fm_ticks

    print(f"timing (frames advanced between two samples {ssf2_ticks} ssf2 ticks / {fm_ticks} fraymakers ticks apart)")
    print(f"  {'element':40}{'ssf2':>10}{'fraymakers':>14}  verdict")
    timing_bad = 0
    for nm, length, sa, fa, labs in rows:
        # Report the CAUSE where it's knowable. An element SSF2 drives across several frame
        # labels only ever plays those segments, so it can't match a conversion that plays the
        # whole timeline linearly — the rate is a symptom, the flattening is the defect.
        if len(labs) > 1 and nm in linear:
            print(f"  {nm[:39]:40}{sa if sa is not None else '-':>10}"
                  f"{fa if fa is not None else '-':>14}  "
                  f"ok — its whole timeline came through; {', '.join(repr(l) for l in sorted(labs))} are marks on it")
            continue
        if len(labs) > 1 and machines.get(nm.split(" \u25b8 ")[0], {}).get("seen"):
            # the port made this clip a state machine and its objects were seen playing sections
            # of it: the two engines were sampled at different moments, and which section each was
            # in is the stage's clock, not the port's shape
            m = machines[nm.split(" \u25b8 ")[0]]
            print(f"  {nm[:39]:40}{sa if sa is not None else '-':>10}"
                  f"{fa if fa is not None else '-':>14}  "
                  f"ok — a state machine over {', '.join(repr(l) for l in sorted(labs))}; "
                  f"its objects were seen in {', '.join(repr(s) for s in sorted(m['seen']))}")
            continue
        if len(labs) > 1:
            # an element the source drives across labels is a state machine, and its port is
            # a game object that plays those labels; it is reproduced when the object was seen
            # playing them, whatever its clock says between two samples
            # the port names a label with its letters and digits only, so both sides are read
            # that way (`good_egg_loop` is `goodeggloop` here)
            key = lambda s: re.sub(r"[^A-Za-z0-9]", "", s).lower()
            played = {key(l) for l in set().union(*fm_cgo_anims.values())} if fm_cgo_anims else set()
            hit = sorted(l for l in labs if key(l) in played)
            # ...or a plane the stage's own entity draws: the port's plane clip was in those sections
            plane = nm.split(" \u25b8 ")[0]
            if not hit and plane in PLANE_LABELS and " \u25b8 " not in nm:
                hit = sorted(l for l in labs if key(l) in {key(x) for x in PLANE_LABELS[plane]})
            if hit:
                print(f"  {nm[:39]:40}{sa if sa is not None else '-':>10}"
                      f"{fa if fa is not None else '-':>14}  "
                      f"ok — a state machine over {', '.join(repr(l) for l in labs)}; "
                      f"seen playing {', '.join(repr(l) for l in hit)} here")
                continue
            timing_bad += 1
            print(f"  {nm[:39]:40}{sa if sa is not None else '-':>10}"
                  f"{fa if fa is not None else '-':>14}  "
                  f"SEGMENTED — ssf2 branches between {', '.join(repr(l) for l in labs)} of "
                  f"{length}f; no Fraymakers object was seen playing any of them")
            continue
        if sa is None or fa is None:
            print(f"  {nm[:39]:40}{'-':>10}{'-':>14}  not measured")
            continue
        # a picture the port's code holds at a frame it picks (a platform's holder by its height)
        # is not a clock loop, whatever two samples of it happen to differ by: a step back of
        # eight frames reads as a lap of sixty. the plan says which pieces those are
        if plan and all(e.get("driven") for e in plan.values() if isinstance(e, dict) and e.get("clip") in (nm, raw)) \
                and any(isinstance(e, dict) and e.get("clip") in (nm, raw) for e in plan.values()):
            print(f"  {nm[:39]:40}{sa:>10}{fa:>14}  driven by the stage's own code here (a frame it picks); reported, not timed")
            continue
        want_len = length * 2 if (length * 2) in fm_c1 else (length - 1) * 2
        s_lin, f_lin = with_clock(sa, ssf2_ticks, length), with_clock(fa, fm_ticks, want_len)
        if s_lin and f_lin:
            note = "ok — steps with the clock on both"
        elif not s_lin and not f_lin:
            note = "driven by its own code on both (not a clock loop); reported, not timed"
        else:
            timing_bad += 1
            note = (f"RATE OFF — steps with the clock on {'ssf2' if s_lin else 'fraymakers'} only "
                    f"(expected {ssf2_ticks % length} / {fm_ticks % max(want_len, 1)})")
        print(f"  {nm[:39]:40}{sa:>10}{fa:>14}  {note}")
    print()
    bad += timing_bad

    # ── placement correlation ───────────────────────────────────────────────────
    # The two engines don't share a coordinate system, and hardcoding a conversion would just
    # bake in an assumption. Instead the mapping is DERIVED from the elements that already
    # matched on animation length: if the port is faithful, one (scale, offset) explains every
    # element, and anything that needs a different one is misplaced. That makes the check
    # self-calibrating — it works on a stage whose coordinate model nobody has written down.
    pairs = []
    claimed = set()
    for length in ssf2_by_len:
        for want in (length * 2, (length - 1) * 2):
            if want in fm_pos and len(fm_pos[want]) == 1:
                names = [n for n, v in ssf2.items() if v == length]
                if len(names) == 1 and names[0] in ssf2_world:
                    pairs.append((names[0], ssf2_world[names[0]], fm_pos[want][0]))
                claimed.add(want)
                break
    # A segmented element is converted to a SUBSET of its timeline, so it can never pair on
    # length. Pair it by elimination instead — its position is just as checkable as anything
    # else's, and dropping it would shrink the placement fit to the point of proving nothing.
    seg_names = [n for n in ssf2 if len(ssf2_labels.get(n.split(" ▸ ")[0], set())) > 1]
    spare = [t for t in fm_pos if t not in claimed and len(fm_pos[t]) == 1]
    if len(seg_names) == 1 and len(spare) == 1 and seg_names[0] in ssf2_world:
        total = ssf2.get(seg_names[0], 0)
        print(f"  (paired by elimination: {seg_names[0]} plays {spare[0]}f in Fraymakers, "
              f"{spare[0] / 2:.0f} of its {total}f timeline)")
        pairs.append((seg_names[0], ssf2_world[seg_names[0]], fm_pos[spare[0]][0]))

    # Tolerance, and why it isn't tighter: an SSF2 bounding box is measured on a LIVE clip, so
    # it changes as the clip animates — the podoboos box was seen at 672x245, 673x253, 704x252
    # and 691x271 across samples of the same object. Placement can therefore only be compared
    # to within an element's own frame-to-frame movement, and a residual smaller than that is
    # not evidence of anything. Sampling both engines on a known frame would tighten it.
    TOL = 40.0

    if len(pairs) >= 2:
        # Least squares over every pair, not a line through two of them: a two-point fit puts
        # all of its error onto whichever point is left out, which reads as "that element is
        # misplaced" when the fit is what's wrong.
        def fit(axis):
            pts = [(sw[axis], fp[axis]) for _n, sw, fp in pairs]
            n = len(pts)
            sx = sum(a for a, _ in pts); sy = sum(b for _, b in pts)
            sxx = sum(a * a for a, _ in pts); sxy = sum(a * b for a, b in pts)
            den = n * sxx - sx * sx
            if abs(den) < 1e-9:
                return None
            k = (n * sxy - sx * sy) / den
            return k, (sy - k * sx) / n
        fx, fy = fit(0), fit(1)
        print("placement (mapping derived from the matched elements)")
        if fx and fy:
            print(f"  fm_x = {fx[0]:.3f} * ssf2_x + {fx[1]:.1f}      "
                  f"fm_y = {fy[0]:.3f} * ssf2_y + {fy[1]:.1f}")
            print(f"  {'element':44}{'ssf2 world':>20}{'fraymakers':>20}{'residual':>12}")
            worst = 0.0
            for n, sw, fp in sorted(pairs):
                px, py = fx[0] * sw[0] + fx[1], fy[0] * sw[1] + fy[1]
                dx, dy = fp[0] - px, fp[1] - py
                r = max(abs(dx), abs(dy))
                worst = max(worst, r)
                flag = "  <-- OFF" if r > TOL else ""
                print(f"  {n[:43]:44}{f'({sw[0]:.0f},{sw[1]:.0f})':>20}"
                      f"{f'({fp[0]:.0f},{fp[1]:.0f})':>20}{r:>11.1f}{flag}")
            print(f"  worst residual {worst:.1f}px "
                  f"({'consistent — one mapping explains every element' if worst <= TOL else 'INCONSISTENT — an element is misplaced'})")
        else:
            print("  not enough separation between elements to derive a mapping")
        print()

    # ── hazards ─────────────────────────────────────────────────────────────────
    # Compared, but SEPARATELY and without a pass/fail on length: a hazard is a state
    # machine (the thwomp has idle/rise/fall/land), so its clock only means something next
    # to the matching state. The two engines are rarely in the same state at sample time —
    # Fraymakers was in `entrance` while SSF2 sat in `idle` — so this reports both sides and
    # leaves the judgement to a state-aware comparison rather than inventing a verdict.
    if ssf2_hazards or fm_objs:
        print("hazards / moving structures (state machines — reported, not length-checked)")
        print(f"  {'ssf2 clip':40}{'frames':>8}  {'state':<18}")
        for name, (total, label) in sorted(ssf2_hazards.items()):
            print(f"  {name[:39]:40}{total:>8}  {label or '-':<18}")
        print(f"  {'fraymakers object':40}{'frames':>8}  {'state':<18}")
        for cls, total, label in sorted(fm_objs):
            print(f"  {cls[:39]:40}{total:>8}  {label:<18}")
        print()

    return bad


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(1 if run(sys.argv[1]) else 0)
