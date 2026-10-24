#!/usr/bin/env python3
"""List calls in emitted scripts that NOTHING can answer, for stages and characters alike.
A call to a name neither this engine nor the script itself has does not fail quietly: it takes
the rest of the frame down with it, every frame. Driving one package at a time finds these one at
a time; the scripts already say which names they use, so the whole output can be asked at once.

Each entity's frame scripts run in the scope of the script its manifest entry names, so they are
read together with it: a frame calling a helper the script defines is answered, a frame calling
one nobody defines is not. Three shapes are checked: `self.name(` against this engine's api and
the scope's own functions, `match.name(` against the api, and a bare `name(` against the scope's
functions, parameters and variables and the globals every script has.

usage: tools/tests/unanswerable-calls.py [converted-dir] [--fail]
  --fail   exit 1 when anything is unanswered (for a sweep or a gate)
"""
import collections, glob, json, os, re, sys

args = [a for a in sys.argv[1:] if not a.startswith("--")]
root = args[0] if args else "characters"
fail = "--fail" in sys.argv
api = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..",
                   "crates", "ssf2-converter", "mappings", "fm_api.jsonc")
known = set()
raw = re.sub(r"^\s*//.*$", "", open(api, encoding="utf-8").read(), flags=re.M)
for names in json.loads(raw).values():
    if isinstance(names, list):
        known |= set(names)

# what every script has without declaring it: the language, and the globals the engine hands in
GLOBALS = {"trace", "Std", "Math", "Type", "Reflect", "StringTools", "Lambda", "Random", "Engine",
           "AudioClip", "match", "self", "Common", "Array", "String", "Int", "Float", "Bool",
           "Dynamic", "haxe", "cast", "untyped", "Sys"}
KEYWORDS = {"if", "for", "while", "switch", "return", "function", "catch", "throw", "new", "else",
            "do", "try", "case", "default", "var", "in", "break", "continue"}

def strip(code):
    """comments out, string bodies blanked, so only real code is read"""
    out, i, n = [], 0, len(code)
    while i < n:
        c = code[i]
        if c == "/" and i + 1 < n and code[i + 1] == "/":
            j = code.find("\n", i)
            i = n if j < 0 else j
            continue
        if c == "/" and i + 1 < n and code[i + 1] == "*":
            j = code.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c in "\"'":
            j = i + 1
            while j < n and code[j] != c:
                j += 2 if code[j] == "\\" else 1
            out.append(c + c)
            i = j + 1
            continue
        out.append(c)
        i += 1
    return "".join(out)

DEF = re.compile(r"\bfunction\s+(\w+)\s*\(")
PARAMS = re.compile(r"\bfunction\s*\w*\s*\(([^()]*)\)")
VAR = re.compile(r"\bvar\s+(\w+)")
ASSIGNED_FN = re.compile(r"(?<![\w.])(\w+)\s*=\s*function\b")
SELF_CALL = re.compile(r"(?<![\w.])self\.(\w+)\s*\(")
MATCH_CALL = re.compile(r"(?<![\w.])match\.(\w+)\s*\(")
BARE_CALL = re.compile(r"(?<![\w.$])(?<!new )([A-Za-z_]\w*)\s*\(")

def defined(code):
    names = set(DEF.findall(code)) | set(VAR.findall(code)) | set(ASSIGNED_FN.findall(code))
    for ps in PARAMS.findall(code):
        for p in ps.split(","):
            p = p.strip().lstrip("?").split(":")[0].split("=")[0].strip()
            if p:
                names.add(p)
    return names

hits = collections.Counter()
where = collections.defaultdict(set)

def check(package, label, code, scope_defs):
    for n in SELF_CALL.findall(code):
        if n not in known and n not in scope_defs:
            hits[f"self.{n}"] += 1; where[f"self.{n}"].add(package)
    for n in MATCH_CALL.findall(code):
        if n not in known:
            hits[f"match.{n}"] += 1; where[f"match.{n}"].add(package)
    for n in BARE_CALL.findall(code):
        if n in KEYWORDS or n in GLOBALS or n in scope_defs or n in known:
            continue
        hits[n] += 1; where[n].add(package)

packages = 0
for manifest in glob.glob(os.path.join(root, "**", "library", "manifest.json"), recursive=True):
    lib = os.path.dirname(manifest)
    package = os.path.basename(os.path.dirname(lib))
    packages += 1
    scripts = {}
    for meta in glob.glob(os.path.join(lib, "scripts", "**", "*.hx.meta"), recursive=True):
        try:
            sid = json.load(open(meta, encoding="utf-8")).get("id")
        except Exception:
            continue
        hx = meta[: -len(".meta")]
        if sid and os.path.exists(hx):
            scripts[sid] = strip(open(hx, encoding="utf-8", errors="replace").read())
    frames = {}
    for ent in glob.glob(os.path.join(lib, "entities", "*.entity")):
        try:
            d = json.load(open(ent, encoding="utf-8"))
        except Exception:
            continue
        codes = [strip(k["code"]) for k in d.get("keyframes", []) if isinstance(k, dict) and k.get("code")]
        if d.get("id"):
            frames[d["id"]] = codes
    try:
        content = json.load(open(manifest, encoding="utf-8")).get("content", [])
    except Exception:
        content = []
    used = set()
    for c in content:
        sid = c.get("scriptId")
        if not sid or sid not in scripts:
            continue
        used.add(sid)
        body = scripts[sid]
        own = frames.get(c.get("id"), [])
        scope = defined(body) | set().union(*(defined(f) for f in own)) if own else defined(body)
        check(package, sid, body, scope)
        for f in own:
            check(package, sid, f, scope)
    # a script nothing names (a follower, a stage helper) is read on its own
    for sid, body in scripts.items():
        if sid not in used:
            check(package, sid, body, defined(body))

for n, c in hits.most_common(40):
    print(f"{c:6}  {n:34} {len(where[n]):3} packages  {sorted(where[n])[:4]}")
total = set().union(*where.values()) if where else set()
print(f"\n{len(hits)} distinct names nothing can answer, across {len(total)} of {packages} packages")
sys.exit(1 if fail and hits else 0)
