#!/usr/bin/env python3
"""knockback_parity.py — does a converted hit launch its victim the way the source does?

Both engines work a launch out the same way, from the same three numbers: how much the hit did,
what the hitbox declares for base and for growth, and how heavy the victim is. That was
established by measuring: Fraymakers' launch is reproduced from its own emitted stats to within
0.01% on every move tried, with one scale for the whole engine.

Which means this needs no engine at all. The source's numbers say what its launch is at any
damage; the emitted numbers say what this one's is; and a converted move is right when the two
curves agree, not merely when they agree at one damage. So each move is checked at several
victim damages and the WORST disagreement is what gets reported.

  source stats  <- DUMP_PARITY=1 peptide convert <char>.ssf   (the SSF2 table, per move)
  emitted stats <- characters/<char>/library/scripts/*/HitboxStats.hx

Usage: tools/tests/knockback_parity.py <char> [<char> …]      (--all for every converted one)
"""
import json, os, re, sys, glob, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# how much of a launch each engine's own units are worth; the RATIO is what matters, and it is
# the same for every character because it is the engine's, not the character's.
FM_SCALE, SSF2_SCALE = 0.17925, 0.21040
WORLD = 0.65        # what a source-world speed is worth here
DAMAGES = (0.0, 30.0, 60.0, 120.0)   # the victim damages a launch is compared at


def launch(base, growth, dealt, before, weight, scale):
    total = before + dealt
    inner = total * 0.1 + dealt * total * 0.05
    return (((inner * 1.4 * (200.0 / (weight + 100.0))) + 18.0) * (growth * 0.01) + base) * scale


def emitted(char):
    """Every emitted hitbox, per move: (damage, base, growth)."""
    out = {}
    for p in glob.glob(os.path.join(ROOT, "characters", char, "library/scripts/*/HitboxStats.hx")):
        t = open(p, encoding="utf-8", errors="replace").read()
        for mv in re.finditer(r'(\w+):\s*\{(.*?)\n\t\}', t, re.S):
            name, body = mv.group(1), mv.group(2)
            boxes = re.findall(
                r'damage:\s*([0-9.]+)[^}]*?baseKnockback:\s*([0-9.]+)[^}]*?knockbackGrowth:\s*([0-9.]+)',
                body, re.S)
            if boxes:
                out[name] = [(float(d), float(b), float(g)) for d, b, g in boxes]
    return out


def weight_of(char):
    for p in glob.glob(os.path.join(ROOT, "characters", char, "library/scripts/*/CharacterStats.hx")):
        m = re.search(r'weight:\s*([0-9.]+)', open(p, encoding="utf-8", errors="replace").read())
        if m:
            return float(m.group(1))
    return 100.0


def source(char):
    p = os.path.join(tempfile.gettempdir(), f"parity_{char}_ssf2.json")
    if not os.path.exists(p):
        return None
    return json.load(open(p))


def check(char, tol=0.03):
    src, emi = source(char), emitted(char)
    if src is None:
        print(f"[{char}] SKIP — no source dump (run: DUMP_PARITY=1 peptide convert {char}.ssf)")
        return None
    w = weight_of(char)
    worst, checked, bad = [], 0, 0
    for move, boxes in sorted(src.items()):
        if move not in emi:
            continue
        for i, sb in enumerate(boxes):
            if i >= len(emi[move]):
                continue
            # a fixed-knockback hit has no counterpart here; it is reported on its own elsewhere
            if float(sb.get("weightKB") or 0):
                continue
            s_base, s_growth = float(sb.get("power") or 0), float(sb.get("kbConstant") or 0)
            s_dmg = float(sb.get("damage") or 0)
            e_dmg, e_base, e_growth = emi[move][i]
            checked += 1
            off = 0.0
            for d in DAMAGES:
                a = launch(s_base, s_growth, s_dmg, d, w, SSF2_SCALE) * WORLD
                b = launch(e_base, e_growth, e_dmg, d, w, FM_SCALE)
                if a > 0:
                    off = max(off, abs(b - a) / a)
            if off > tol:
                bad += 1
                worst.append((off, f"{move}[{i}]", s_base, s_growth, e_base, e_growth))
    worst.sort(reverse=True)
    if bad == 0:
        print(f"[{char}] KNOCKBACK OK — {checked} hitbox(es), every one within {tol:.0%} at every damage")
    else:
        print(f"[{char}] {bad} of {checked} hitbox(es) off by more than {tol:.0%}:")
        for off, name, sb, sg, eb, eg in worst[:6]:
            print(f"    {name:28} off {off:6.1%}   source {sb:g}/{sg:g}  emitted {eb:g}/{eg:g}")
    return bad, checked


def main():
    args = sys.argv[1:]
    if args == ["--all"]:
        args = sorted(os.path.basename(os.path.dirname(p))
                      for p in glob.glob(os.path.join(ROOT, "characters", "*", "library")))
    if not args:
        print(__doc__); sys.exit(2)
    tb = tc = 0
    for c in args:
        r = check(c)
        if r:
            tb += r[0]; tc += r[1]
    print(f"\nTOTAL: {tc - tb} of {tc} hitboxes launch the way the source does")
    sys.exit(1 if tb else 0)


main()
