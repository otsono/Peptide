#!/usr/bin/env python3
"""Read every script a conversion emitted and say what the engine would refuse to parse.

A script the engine cannot parse fails silently: the whole file is dropped and the first
symptom is a frame reporting an unknown variable for a name that is right there in it. These
are the shapes that have caused that, each one found the hard way.

usage: tools/check-emitted.py [characters_dir]
"""
import glob, json, os, re, sys

root = sys.argv[1] if len(sys.argv) > 1 else "characters"

def scripts():
    # ANY depth under the root. A sweep that gives each stage its own output directory nests one
    # level deeper than a single conversion does, and a glob written for one silently matches
    # nothing in the other -- which reads as "no problems" and is the most misleading answer a
    # checker can give.
    for p in glob.glob(os.path.join(root, "**", "library", "scripts", "**", "*.hx"), recursive=True):
        yield p, open(p, encoding="utf-8", errors="replace").read()
    for p in glob.glob(os.path.join(root, "**", "library", "entities", "*.entity"), recursive=True):
        try:
            d = json.load(open(p, encoding="utf-8"))
        except Exception:
            continue
        for k in d.get("keyframes", []):
            if isinstance(k, dict) and k.get("code"):
                yield f"{p}#{k.get('$id')}", k["code"]

def strip(code):
    """Drop comments and string bodies so counting brackets counts real ones."""
    out, i, n = [], 0, len(code)
    while i < n:
        c = code[i]
        if c == "/" and i + 1 < n and code[i + 1] == "/":
            while i < n and code[i] != "\n": i += 1
        elif c == "/" and i + 1 < n and code[i + 1] == "*":
            i = code.find("*/", i + 2)
            i = n if i < 0 else i + 2
        elif c in "\"'":
            q = c; i += 1
            while i < n and code[i] != q:
                i += 2 if code[i] == "\\" else 1
            i += 1
        else:
            out.append(c); i += 1
    return "".join(out)

fails = []
for name, code in scripts():
    bare = strip(code)
    if bare.count("{") != bare.count("}"):
        fails.append((name, f"braces {bare.count('{')} open vs {bare.count('}')} close"))
    # A CLOSE THAT ARRIVES EARLY. Counting the file over says nothing about where the braces are:
    # DAT61 balanced exactly, and still would not parse, because a stray `}` ended a function
    # early and left the function's own `}` as a token nothing expected. Depth is what the parser
    # actually tracks, so track it -- the first line that takes it below zero is the one to look at.
    depth, opened = 0, []
    for i, l in enumerate(bare.split("\n"), 1):
        for c in l:
            if c == "{":
                depth += 1; opened.append(i)
            elif c == "}":
                depth -= 1
                if opened: opened.pop()
                if depth < 0:
                    fails.append((name, f"line {i}: a close with nothing open for it"))
                    break
        if depth < 0: break
    if depth > 0 and opened:
        fails.append((name, f"line {opened[0]}: a block that never closes"))
    if bare.count("(") != bare.count(")"):
        fails.append((name, f"parens {bare.count('(')} open vs {bare.count(')')} close"))
    if bare.count("[") != bare.count("]"):
        fails.append((name, f"brackets {bare.count('[')} open vs {bare.count(']')} close"))
    # an index access with a dot in front of it, from a rewrite that replaced the tail of a
    # qualified name and left the qualifier behind. parses nowhere, and the whole script goes.
    m = re.search(r"\.\s*\[", bare)
    if m: fails.append((name, "an index reached through a dot"))
    # a name run into the call that follows it, from a rename that stopped short
    m = re.search(r"\)[A-Za-z_][A-Za-z_0-9]*\s*\(", bare)
    if m: fails.append((name, f"a call run into what precedes it: {m.group(0)!r}"))
    # a statement whose own brackets do not close on its own line, from a split that cut through
    # a call. the file still balances, so only the line shows it.
    for i, l in enumerate(bare.split("\n"), 1):
        t = l.strip()
        # a line that only CLOSES things is not a statement: `});` ends a function handed to a
        # call, and its brackets were opened lines above
        closer_only = t.strip("});").strip() == ""
        # only an EXCESS OF OPENS is a cut call: a statement ending in `;` that opened more
        # parens than it closed lost its tail. a line that closes more than it opens is a
        # legitimate continuation-close (`}, { persistent: true });` ends an anonymous callback
        # and its options object handed to a call opened lines above), not a broken statement.
        if (t.endswith(";") and not closer_only and not t.startswith("for")
                and t.count("(") > t.count(")")):
            fails.append((name, f"line {i}: a statement whose brackets do not close on it"))
            break
    # a value welded to what precedes it, from a replacement that started in the middle of the
    # thing it was replacing
    m = re.search(r"[\)\]][0-9]", bare)
    if m: fails.append((name, f"a number run into what precedes it: {m.group(0)!r}"))
    # a typed catch (`catch (err:Dynamic)`) parses: a fighter's script carrying one loads and the
    # catch catches (measured live on krystal's type ask), so it is not a problem here
    # a receiver left with nothing to call on it
    m = re.search(r"[A-Za-z_0-9)]\.\s*\(", bare)
    if m: fails.append((name, f"a call with no name: {m.group(0)!r}"))
    # a guard wrapped around a fragment rather than a whole statement
    for i, l in enumerate(bare.split("\n"), 1):
        g = re.match(r"\s*if \(\w+\.get\(\) != null\) \{ (.*) \}$", l)
        if g and g.group(1).count("(") != g.group(1).count(")"):
            fails.append((name, f"line {i}: a guard around a fragment"))

for n, why in fails[:40]:
    print(f"{n}: {why}")
print(f"{len(fails)} problem(s) across the emitted scripts")
sys.exit(1 if fails else 0)
