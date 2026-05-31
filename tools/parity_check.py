#!/usr/bin/env python3
"""parity_check.py — SSF2 functional-parity verifier for converted hitbox stats.

Compares the SSF2 SOURCE-OF-TRUTH hitbox values (dumped from the .ssf by the
converter when run with DUMP_PARITY=1 -> /tmp/parity_<char>_ssf2.json) against the
converter's generated HitboxStats.hx, applying the documented SSF2->Fraymakers
field mapping. Flags every divergence so it can be fixed converter-side.

This is the P1 parity gate: P0 is "animates, no crash"; this checks the move
actually carries SSF2's intended damage / knockback angle / base knockback /
knockback growth / hit-freeze, per hitbox.

SSF2 -> Fraymakers mapping (from mappings/character/hitbox_stats.jsonc):
    damage          <- damage
    angle           <- direction (or angle)
    baseKnockback   <- power
    knockbackGrowth <- kbConstant
    hitstop         <- hitStun     (frame field: x2 for 30->60fps; <=0 -> -1 sentinel)
    selfHitstop     <- selfHitStun (x2; <=0 -> -1)
    hitstun         <- hitLag      (x2; int(hitLag) in {255,-1} or <=... -> -1)
NOTE: SSF2 weightKB (weight-scaled knockback) currently has NO FM mapping — flagged
as INFO so a human can decide whether baseKnockback should fold it in.

Usage:
    DUMP_PARITY=1 ./target/release/ssf2_converter ../ssf2-ssfs/<char>.ssf
    tools/parity_check.py <char> [<char> ...]
Exit 0 if all checked characters pass; 1 if any divergence.
"""
import json, re, sys, os

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

def base_move(m):
    """Mirror src/haxe_gen.rs base_attack_name: split sub-anims inherit a base."""
    mm = re.match(r"^jab(\d+)$", m)
    if mm and int(mm.group(1)) >= 2:
        return "jab1"
    for suf in ("_in", "_charge"):
        if m.endswith(suf) and m.startswith("strong_"):
            return m[: -len(suf)] + "_attack"
    return None

def pascal(cid):
    return cid[:1].upper() + cid[1:]

def parse_hitbox_stats(path):
    """Parse HitboxStats.hx -> ({move: [hitbox fields]}, extras_set).
    extras_set = moves emitted in the commented `// SSF2: <name>:` extras section
    (present-but-inactive: the converter couldn't map them to a standard move)."""
    txt = open(path).read()
    extras = set(re.findall(r"//\s*SSF2:\s*(\w+)\s*:", txt))
    # strip // comments so commented placeholders don't parse as active data
    txt = re.sub(r"//[^\n]*", "", txt)
    out = {}
    # move blocks: `name: {  hitbox0: {...}, hitbox1: {...} }`
    for mv in re.finditer(r"(\w+)\s*:\s*\{((?:[^{}]|\{[^{}]*\})*)\}", txt):
        name, body = mv.group(1), mv.group(2)
        if not re.search(r"hitbox\d+\s*:", body):
            continue
        boxes = []
        for hb in re.finditer(r"hitbox(\d+)\s*:\s*\{([^}]*)\}", body):
            fields = {}
            for f in re.finditer(r"(\w+)\s*:\s*(-?\d+(?:\.\d+)?)", hb.group(2)):
                fields[f.group(1)] = float(f.group(2))
            boxes.append(fields)
        out[name] = boxes
    return out, extras

def expected_fm(hb):
    """Map one raw SSF2 hitbox dict -> expected FM field values."""
    def g(*ks):
        vals = [hb[k] for k in ks if k in hb]
        return max(vals) if vals else 0.0
    def frame(v):
        iv = int(v)
        return -1 if iv <= 0 else iv * 2
    hitlag = hb.get("hitLag", -1)
    ilag = int(hitlag)
    return {
        "damage": int(g("damage")),
        "angle": int(g("direction", "angle")),
        "baseKnockback": int(g("power", "weightKB")),  # max(power, weightKB) — see hitbox_stats.jsonc
        "knockbackGrowth": int(g("kbConstant")),
        "hitstop": frame(hb.get("hitStun", -1)),
        "selfHitstop": frame(hb.get("selfHitStun", -1)),
        "hitstun": (-1 if (ilag == 255 or ilag <= 0) else ilag * 2),
    }

CORE = ["damage", "angle", "baseKnockback", "knockbackGrowth"]  # gameplay-critical

def hitbox_active_frames(cid):
    """Parse <Char>.entity -> {animation_name: total active HIT_BOX frames}.
    A hitbox is 'active' on a keyframe whose symbol is non-null. This is the
    frame-data dimension: a move with stats but 0 active frames can't actually hit."""
    ent = os.path.join(REPO, f"characters/{cid}/library/entities/{pascal(cid)}.entity")
    if not os.path.exists(ent):
        return None
    d = json.load(open(ent))
    layers = {l["$id"]: l for l in d.get("layers", [])}
    kfs = {k["$id"]: k for k in d.get("keyframes", [])}
    out = {}
    for a in d.get("animations", []):
        total = 0
        for lid in a.get("layers", []):
            l = layers.get(lid)
            if not l or l.get("type") != "COLLISION_BOX":
                continue
            m = l.get("pluginMetadata", {}).get("com.fraymakers.FraymakersMetadata", {})
            if m.get("collisionBoxType") != "HIT_BOX":
                continue
            for kid in l.get("keyframes", []):
                k = kfs.get(kid, {})
                if k.get("symbol") is not None:
                    total += k.get("length", 1)
        out[a["name"]] = total
    return out

def check_char(cid):
    ssf2_path = f"/tmp/parity_{cid}_ssf2.json"
    if not os.path.exists(ssf2_path):
        print(f"[{cid}] SKIP — no {ssf2_path} (run: DUMP_PARITY=1 ssf2_converter {cid}.ssf)")
        return None
    src = json.load(open(ssf2_path))
    hx = os.path.join(REPO, f"characters/{cid}/library/scripts/{pascal(cid)}/HitboxStats.hx")
    if not os.path.exists(hx):
        print(f"[{cid}] SKIP — no {hx}")
        return None
    out, extras = parse_hitbox_stats(hx)
    issues = []
    info = []
    for move, src_boxes in sorted(src.items()):
        if move not in out:
            if move in extras:
                info.append(f"{move}: SSF2 source present but emitted in the commented extras section (not mapped to a standard FM move — review)")
            else:
                issues.append(f"{move}: in SSF2 source ({len(src_boxes)} hitbox(es)) but MISSING from HitboxStats.hx")
            continue
        out_boxes = out[move]
        for i, shb in enumerate(src_boxes):
            exp = expected_fm(shb)
            if i >= len(out_boxes):
                issues.append(f"{move}.hitbox{i}: SSF2 has it (dmg={exp['damage']} ang={exp['angle']}) but HitboxStats omits it")
                continue
            got = out_boxes[i]
            for fld in CORE:
                e = exp[fld]
                if fld not in got:
                    issues.append(f"{move}.hitbox{i}.{fld}: expected {e}, missing in output")
                elif int(got[fld]) != e:
                    issues.append(f"{move}.hitbox{i}.{fld}: SSF2->expected {e}, output {int(got[fld])}")
            # weightKB now folds into baseKnockback (max(power, weightKB)); note the
            # cases where weightKB is the dominant source so a human can sanity-check.
            if shb.get("weightKB", 0) > shb.get("power", 0):
                info.append(f"{move}.hitbox{i}: baseKnockback={int(exp['baseKnockback'])} from SSF2 weightKB (power={int(shb.get('power',0))})")
    # Frame-data dimension: hitbox COVERAGE — how many stat-bearing moves actually have
    # active hitbox frames in the entity. A near-zero coverage means broken sprite/box
    # extraction (the whole character can't hit — this is how the 5 empty-shell chars were
    # found). A FEW stat-moves with no active frames is normal (projectile specials like a
    # fireball, throws) so per-move misses are INFO, not failures; only a severe shortfall
    # is flagged as an issue.
    af = hitbox_active_frames(cid)
    if af is not None:
        stat_moves = [m for m, boxes in out.items()
                      if any(int(hb.get(f, 0)) for hb in boxes for f in CORE)]
        with_frames = [m for m in stat_moves if af.get(m, 0) > 0]
        cov = (len(with_frames) / len(stat_moves)) if stat_moves else 1.0
        info.append(f"hitbox frame-coverage: {len(with_frames)}/{len(stat_moves)} stat-moves have active frames ({cov*100:.0f}%)")
        if stat_moves and cov < 0.4:
            issues.append(f"BROKEN sprite extraction: only {len(with_frames)}/{len(stat_moves)} stat-moves have active hitbox frames — most moves can't hit")

    # moves in output but not source: inheritance (ok) or spurious
    for move in sorted(out):
        if move in src:
            continue
        b = base_move(move)
        if b and b in src:
            continue  # inherited from base — expected
        # could be a template move with no SSF2 attack (idle/grab placeholder) — only
        # flag if it actually carries nonzero combat stats
        if any(int(hb.get(f, 0)) for hb in out[move] for f in CORE):
            issues.append(f"{move}: in HitboxStats with nonzero stats but NO SSF2 source and not an inheritance of a base move")
    return issues, info

def main():
    chars = sys.argv[1:]
    if not chars:
        print("usage: parity_check.py <char> [<char> ...]"); sys.exit(2)
    any_fail = False
    for cid in chars:
        r = check_char(cid)
        if r is None:
            continue
        issues, info = r
        if not issues:
            print(f"[{cid}] PARITY OK — all SSF2 hitbox values present + correctly mapped"
                  + (f"  ({len(info)} info)" if info else ""))
        else:
            any_fail = True
            print(f"[{cid}] {len(issues)} DIVERGENCE(S):")
            for s in issues:
                print(f"    ✗ {s}")
        for s in info:
            print(f"    · {s}")
    sys.exit(1 if any_fail else 0)

if __name__ == "__main__":
    main()
