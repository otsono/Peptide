#!/usr/bin/env python3
"""stage_inventory.py — map EVERY object on a stage, SSF2 side against Fraymakers side.

PORTING_STAGES phase 1 says "the goal: a complete list of every symbol, plane, actor,
bitmap, and clip before you form any opinion about the stage", and phase 4 says the live
tree "catches everything the file can't tell you". That procedure exists as a set of debug
dumps you read by eye. This turns the static half into a paired inventory you can diff, so
"did we port every object" stops being a judgement call.

SSF2 side  <- PEPTIDE_STAGE_TREE: every placed instance with depth, kind (MC/shape),
              instance name, symbol/linkage, resolved PLANE (the layer), and world position.
Fraymakers <- the emitted stage package: the main entity's layers (art, collision, spawn
              points) plus every separately-emitted entity (backdrop elements, hazards),
              each with its animation count and frame count.

Reported per object: name, plane/layer, position, and on the FM side the animation and
frame count it became.

READ "NO COUNTERPART" CAREFULLY. It does not mean "not ported". The stage's art is
RASTERIZED AND COMPOSITED into one sprite (STATUS "stage porting"), so a STATIC object is
correctly present as pixels in that sprite while having no named object of its own. What
actually matters is the ANIMATED objects: those need their own layer/entity to keep
playing, and one baked into the composite has silently lost its animation. So a
no-counterpart row is a QUESTION — "does this thing move?" — not a defect.

Objects that are never art (collision, masks, spawn/boundary scaffolding, container clips,
HUD chrome, hazard attackBoxes) are classified out; see AGENT_CONTEXT's "SSF2 stage linkage
vocabulary".

BLIND SPOT, and it is a real one: this only sees TIMELINE animation, i.e. a clip with more
than one frame. An object animated by AS3 (weather systems, spawned actors) is a 1-frame
clip and reads here as correctly-baked static art. bowserscastle's `bowsers_embers_bg` is
exactly that — 1 frame, driven by an EmberWeather class, and a documented unported gap that
this check cannot see. The actor/class axis is PORTING_STAGES phase 2, not this inventory;
the two are complementary and neither alone is a clean bill of health.

Observed on a 4-stage sample: kingdom1 loses 4 animations, all on the `cambg` parallax
plane (clouds 500 frames, bouncy 360, lefttroopa/righttroopa 263 each). bowserscastle,
casinonightzone and battlefield lose none.

Usage:
  tools/tests/stage_inventory.py <stage>            # e.g. bowserscastle
  tools/tests/stage_inventory.py <stage> --all      # include unnamed shape leaves
"""
import json, os, re, subprocess, sys, glob, collections

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SSFS = os.environ.get("SSF2_SSFS_DIR", os.path.join(ROOT, "..", "ssf2-ssfs"))
BIN = os.path.join(ROOT, "build", "release", "peptide")

# Linkage families that are deliberately NOT art (AGENT_CONTEXT "SSF2 stage linkage
# vocabulary"): collision geometry the engine renders invisibly, and scaffolding.
NON_ART = re.compile(
    # `stance` is an object's OWN ART, which the converter classifies out for that reason -- and
    # counting it here as an object of its own made it compete for an element with the object it
    # belongs to, so a target test's tenth target was reported lost to its neighbour's stance.
    r"^stance$|terrain|platform|collison|collision|ledge_mc|_start_|_spawn_|boundary|"
    r"warningbounds|warning|itemgen|mask|light_source|smashball|"
    # wrappers and hazard collision, not art: `stageMC`/`stance` are container clips and
    # `attackBox*` becomes the hazard's HITBOX, not a drawn object.
    r"^stagemc$|^stance$|attackbox|hitbox|"
    # HUD furniture is chrome, not stage art; `weightPlat*`-style moving platforms are
    # kept as static COLLISION (see STATUS "auto-detected moving platforms"), not as a
    # drawn object, so their absence from the art inventory is correct.
    r"hud|weightplat",
    re.I)


def convert_once(stage, outdir):
    """ONE converter run that both emits the package and dumps the placement tree.

    The first version ran the converter twice per stage (once with --info for the tree,
    once with --out to emit), which doubled a 110-stage sweep for nothing.
    """
    src = os.path.join(SSFS, "stages", f"{stage}.ssf")
    env = dict(os.environ, PEPTIDE_STAGE_TREE="1")
    r = subprocess.run([BIN, "ssf2", "stage", src, "--out", outdir],
                       capture_output=True, text=True, env=env)
    return r.stderr + r.stdout


def ssf2_objects(text, include_unnamed=False):
    """Every placed instance from the SSF2 placement tree."""
    r = None
    out = []
    for m in re.finditer(
            r'^(\s*)d(\d+) (\S+) inst="([^"]*)" sym="([^"]*)" plane="([^"]*)" '
            r'@\((-?\d+),(-?\d+)\) frames=(\d+)',
            text, re.M):
        indent, depth, kind, inst, sym, plane, x, y, frames = m.groups()
        name = inst or sym
        if not name and not include_unnamed:
            continue
        out.append({"depth": int(depth), "kind": kind, "name": name, "inst": inst,
                    "sym": sym, "plane": plane, "x": int(x), "y": int(y),
                    "frames": int(frames), "nest": len(indent) // 2})
    return out


def spawn_layers(stage, outdir):
    """`<entity id>` -> the stage container its spawn line reparents it into.

    A separately-emitted object (a backdrop vfx, a hazard) is placed by a line in the stage
    Script, not by anything in its own `.entity`, so its DEPTH only exists in that script.
    Fraymakers has eleven named containers; reading which one each object actually lands in
    is what makes a wrong plane visible instead of invisible.
    """
    sid = f"{stage}ssf2"
    script = os.path.join(outdir, sid, "library", "scripts", "stage", f"{sid}Script.hx")
    out = {}
    if not os.path.exists(script):
        return out
    text = open(script, encoding="utf8").read()
    # var _bg3 = match.createVfx(... getContent("<id>") ...);
    #   if (_bg3 != null) { self.getBackgroundBehindContainer().addChild(_bg3.getSprite()); }
    for var, ent in re.findall(r'var (_\w+) = match\.create\w+\([^;]*?getContent\("([^"]+)"\)', text):
        m = re.search(re.escape(var) + r'\s*!=\s*null\s*\)\s*\{\s*self\.get(\w+?)Container\(\)', text)
        if m:
            # getBackgroundBehindContainer -> BACKGROUND_BEHIND
            name = re.sub(r'(?<!^)(?=[A-Z])', '_', m.group(1)).upper()
            out[ent] = name
        else:
            out.setdefault(ent, "(game objects)")
    return out


def fm_objects(stage, outdir):
    """Every emitted Fraymakers object: main-entity layers + separate entities."""
    sid = f"{stage}ssf2"
    base = os.path.join(outdir, sid, "library", "entities")
    spawned = spawn_layers(stage, outdir)
    objs = []
    main = os.path.join(base, f"{sid}.entity")
    if os.path.exists(main):
        d = json.load(open(main, encoding="utf8"))
        layers = {l["$id"]: l for l in d["layers"]}
        kfs = {k["$id"]: k for k in d["keyframes"]}
        for a in d["animations"]:
            for lid in a["layers"]:
                l = layers.get(lid)
                if not l:
                    continue
                frames = sum(kfs[k].get("length", 1) for k in l.get("keyframes", []) if k in kfs)
                objs.append({"where": "main", "anim": a["name"], "name": l.get("name") or "",
                             "type": l.get("type"), "frames": frames,
                             "layer": "(baked in stack)"})
    for ent in sorted(glob.glob(os.path.join(base, "*.entity"))):
        if os.path.basename(ent) == f"{sid}.entity":
            continue
        d = json.load(open(ent, encoding="utf8"))
        layers = {l["$id"]: l for l in d["layers"]}
        kfs = {k["$id"]: k for k in d["keyframes"]}
        nm = os.path.basename(ent)[:-len(".entity")]
        frames = 0
        for a in d.get("animations", []):
            for lid in a["layers"]:
                l = layers.get(lid)
                if l and l.get("type") == "IMAGE":
                    frames = max(frames, sum(kfs[k].get("length", 1)
                                             for k in l.get("keyframes", []) if k in kfs))
        objs.append({"where": "entity", "anim": ",".join(a["name"] for a in d.get("animations", [])),
                     "name": nm, "type": "ENTITY", "frames": frames,
                     "layer": spawned.get(nm, "(not spawned)")})
    return objs


# SSF2 framework / Flash-generated classes are not stage content. `*_fla.*` are timeline
# symbols the placement tree already covers; the rest are engine plumbing.
FRAMEWORK = re.compile(r"^(SSF2|IState|EState|FrameTimer|ItemSettings|CState|PState|TState|"
                       r"ControlBits|Main|Vec2|Point|Rect)", re.I)


def actor_classes(stage):
    """Hand-written AS3 classes in the stage file — its behavioural objects.

    This is the blind spot the placement tree cannot cover: a weather system or spawned
    actor is a CLASS, not a placed multi-frame clip, so it reads as 1-frame static art (or
    doesn't appear at all). PORTING_STAGES phase 2 calls for exactly this inventory.
    """
    src = os.path.join(SSFS, "stages", f"{stage}.ssf")
    # RUN THE TOOL, DO NOT ASK CARGO TO. `cargo run` re-resolves the workspace on every call, and
    # at one call per stage that was 29 of the 31 seconds this check took -- an hour for the
    # corpus, nearly all of it cargo deciding there was nothing to rebuild. Built once, invoked
    # directly, the same sweep is a couple of minutes.
    exe = next((c for c in (os.path.join(ROOT, "build", p, "ssf2_objgraph")
                            for p in ("release", "debug")) if os.path.exists(c)), None)
    cmd = ([exe, src, "scripts"] if exe else
           ["cargo", "run", "-q", "-p", "ssf2_converter", "--features", "dev-tools",
            "--bin", "ssf2_objgraph", "--", src, "scripts"])
    r = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
    out = []
    for m in re.finditer(r"^  class  (\S+)", r.stdout + r.stderr, re.M):
        c = m.group(1)
        if "_fla." in c or FRAMEWORK.match(c) or c.lower() == stage.lower():
            continue
        if c.lower() == f"{stage}_bg".lower():
            continue
        # Hand-written ACTORS are PascalCase (BowsersCastleLava, EmberWeather, Thwomp).
        # snake_case entries (global_wind_wave, podoboo_jump, stage_bowserscastle) are
        # exported LIBRARY SYMBOLS — timeline content the placement tree already accounts
        # for, not behaviour classes. Listing them here reports the same object twice and
        # buries the one finding that matters.
        if not re.match(r"^[A-Z][A-Za-z0-9]*$", c):
            continue
        out.append(c)
    return sorted(set(out))


def provenance(stage, outdir):
    """What the converter says each emitted element came from.

    Pairing the two sides by NAME cannot work on its own: an element is named for the symbol it
    came out of, and a symbol id is not the name the tree shows. The converter knows the pairing
    exactly, so it writes it down, and this reads it. `frames` is what was EMITTED after
    run-length encoding, so an element that came out as one picture says so -- and a source clip
    of a thousand frames whose art never changes is correctly one picture, not a loss.
    """
    p = os.path.join(outdir, f"{stage}ssf2", f"{stage}ssf2.sources.json")
    if not os.path.exists(p):
        return {}
    try:
        d = json.load(open(p))
    except Exception:
        return {}
    return d.get("elements", [])


def drawn_by_object(stage, outdir, name):
    """The object that draws this clip, if one does.

    A hazard is emitted as an entity named for its CLASS, while the source knows it by the clip
    it draws with. Emerald cave's wooden panel is ten labelled frames -- states, not a loop --
    ported as an object with ten states and reported as a lost animation, because `ecave_wood`
    and `EmeraldCaveWoodenPanel` share no letters to pair on. The converter writes the link.
    """
    p = os.path.join(outdir, f"{stage}ssf2", f"{stage}ssf2.sources.json")
    if not os.path.exists(p):
        return None
    try:
        d = json.load(open(p))
    except Exception:
        return None
    for o in d.get("objects", []):
        if o.get("draws") and o["draws"] == name:
            return o
    return None


def pair_elements(prov, rows):
    """Pair each animated source object with the emitted element that carries it.

    Done for ALL of them at once rather than one at a time, because one at a time is decided by
    the order the objects happen to be listed in: an early object takes an element an exact match
    later on needed, and the report then blames the later one. Three passes, surest first --
    the emitter's own note of what it came from, then equal length, then anything still moving.

    A track is sampled over the timeline it is PLACED on, so a 49-frame loop on a 94-frame parent
    comes out 94 long with the loop shown twice: the same thing to look at, a different number to
    match on. What decides whether movement survived is that an object that moves has an element
    that moves, and that each element answers for one object only.
    """
    # what is never an object of its own does not get to hold an element: a stance is the art of
    # the object it hangs under, and letting it compete meant a target test's tenth target was
    # reported lost to its neighbour's stance.
    rows = [o for o in rows if not NON_ART.search(o["name"])]
    out, used = {}, set()
    def take(o, e):
        out[o["name"]] = e
        used.add(id(e))
    for o in rows:
        for e in prov:
            if id(e) not in used and e.get("from") == o["name"]:
                take(o, e); break
    for o in rows:
        if o["name"] in out: continue
        for e in prov:
            if id(e) in used or e.get("frames", 0) <= 1: continue
            if e["frames"] == o["frames"]:
                take(o, e); break
    for o in rows:
        if o["name"] in out or o.get("frames", 0) <= 1: continue
        for e in prov:
            if id(e) in used or e.get("frames", 0) <= 1: continue
            take(o, e); break
    return out


def norm(s):
    """Loose key for pairing: the emitted names carry the stage id and separators vary."""
    return re.sub(r"[^a-z0-9]", "", s.lower())


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        sys.exit(2)
    stage = args[0]
    include_unnamed = "--all" in sys.argv

    # One directory per run when asked, so a whole-corpus sweep can put the stages across the
    # machine's cores without ten converters writing over each other's output.
    outdir = os.environ.get("PEPTIDE_STAGE_OUT") or os.path.join(ROOT, "build", "stage_inventory")
    if not os.path.isabs(outdir):
        outdir = os.path.join(ROOT, outdir)
    text = convert_once(stage, outdir)
    ssf2 = ssf2_objects(text, include_unnamed)
    fm = fm_objects(stage, outdir)
    sid = norm(stage + "ssf2")

    # index the FM side by normalized name with the stage id stripped
    # Index by BOTH the layer name and the animation name: a main-entity object can be
    # identified by either (the platform sprites are animations named platformSprite0..3
    # whose layers are generically named), and indexing only one silently loses the match.
    fm_index = collections.defaultdict(list)
    for o in fm:
        for raw in {o["name"], o.get("anim", "")}:
            if not raw:
                continue
            k = norm(raw).replace(sid, "").replace(norm(stage), "")
            if k:
                fm_index[k].append(o)

    named = [o for o in ssf2 if o["name"]]
    # one row per distinct SSF2 object name (the tree repeats a clip per frame placement)
    seen, rows = set(), []
    for o in named:
        if o["name"] in seen:
            continue
        seen.add(o["name"])
        rows.append(o)

    print(f"\n=== {stage}: {len(rows)} distinct SSF2 objects, {len(fm)} emitted Fraymakers objects ===\n")
    print(f"{'SSF2 object':30}{'plane':11}{'pos':>13}  {'->':3} {'Fraymakers':34}{'frames':>7}  {'fm layer':<22}")
    print("-" * 112)
    ported = gaps = skipped = 0
    missing = []
    prov = provenance(stage, outdir)
    paired = pair_elements(prov, rows)
    for o in rows:
        key = norm(o["name"]).replace(sid, "").replace(norm(stage), "")
        # match both directions: SSF2 `thwomp_mc` emits as `…Thwomp`, so neither name
        # contains the other in a fixed order once normalized.
        hits = fm_index.get(key) or [h for k, v in fm_index.items()
                                     if key and k and (key in k or k in key) for h in v]
        pos = f"({o['x']},{o['y']})"
        if hits:
            ported += 1
            h = hits[0]
            tag = f"{h['where']}:{h['name'][:30]}"
            fm_layer = h.get("layer", "")
            # flag a background-plane object that landed in a foreground container, or the
            # reverse: that's a depth error the counts alone can't show
            sp, fl = (o['plane'] or '').lower(), fm_layer.lower()
            # A STRUCTURES container is where structures live, not a statement about depth. A
            # platform goes in one whatever plane the source drew it on, so reading
            # `FOREGROUND_STRUCTURES` as "this landed in the foreground" called every ported
            # platform a depth error -- all eleven of them, across seven stages, every one a
            # platform and none of them wrong.
            mismatch = (("fore" in sp) != ("foreground" in fl)) \
                and fl.startswith(("back", "fore", "char")) \
                and "structures" not in fl
            flag = "  <-- PLANE/LAYER MISMATCH" if mismatch else ""
            print(f"{o['name'][:29]:30}{o['plane'][:10]:11}{pos:>13}  ->  {tag:34}{h['frames']:>7}  {fm_layer:<22}{flag}")
        elif (obj := drawn_by_object(stage, outdir, o["name"])):
            ported += 1
            print(f"{o['name'][:29]:30}{o['plane'][:10]:11}{pos:>13}  ->  "
                  f"{'object:' + obj['entity'][:27]:34}{'states':>7}  (its own object)")
        elif o["name"] in paired:
            # THE CONVERTER SAYS WHERE THIS WENT, so no guess is needed. An element it emitted as
            # ONE picture is one of two things and the difference matters: a clip whose art never
            # changes over a long timeline is correctly one picture, and a clip whose art does
            # change is a loss. Only the emitter can tell them apart, because only it rendered
            # them -- it run-length encodes what it drew, so one frame out means one image drawn.
            e = paired[o["name"]]
            ported += 1
            print(f"{o['name'][:29]:30}{o['plane'][:10]:11}{pos:>13}  ->  "
                  f"{'element:' + e['element'][:26]:34}{e['frames']:>7}  (own loop)")
        elif NON_ART.search(o["name"]):
            skipped += 1   # collision geometry / scaffolding: never art, correctly absent
        elif o["frames"] > 1:
            # ANIMATED and not emitted as its own object: baked into the composite sprite,
            # so it renders but no longer moves. This is the actual defect.
            gaps += 1
            missing.append(o)
        else:
            skipped += 1  # 1 frame: static art, correctly baked into the stage sprite
    print("-" * 112)
    print(f"ported={ported}  non-art or static-baked={skipped}  LOST ANIMATION={gaps}\n")

    # ── behavioural classes (the placement tree's blind spot) ─────────────────
    acts = actor_classes(stage)
    if acts:
        print(f"AS3 behaviour classes ({len(acts)}):")
        act_missing = []
        for c in acts:
            k = norm(c)
            hit = [h for kk, v in fm_index.items() if kk and (k in kk or kk in k) for h in v]
            # a *Platform class is emitted as the platformSprite* family, not by name
            if not hit and k.endswith("platform"):
                hit = [h for kk, v in fm_index.items() if "platformsprite" in kk for h in v]
            if hit:
                print(f"  {c[:38]:40}-> {hit[0]['where']}:{hit[0]['name'][:34]}")
            else:
                act_missing.append(c)
        for c in act_missing:
            print(f"  {c[:38]:40}NO COUNTERPART")
        print(f"  ({len(acts) - len(act_missing)} ported, {len(act_missing)} unported)\n")
        gaps += len(act_missing)
    if missing:
        print("ANIMATED SSF2 objects with no emitted counterpart (baked in, so they no longer move):")
        for o in sorted(missing, key=lambda o: -o["frames"]):
            print(f"  {o['name'][:38]:40}plane={o['plane'] or '?':10}frames={o['frames']:>5}  @({o['x']},{o['y']})")
    sys.exit(1 if gaps else 0)
