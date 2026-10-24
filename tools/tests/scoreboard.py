#!/usr/bin/env python3
"""scoreboard.py -- the converter measured on everything the harness can measure, per fighter.

the gates say a fighter converts, boots, holds its physics and plays each move for the right
length. a match is more than that. this runs every measurement the harness has on one fighter
after another, on one conversion path, and keeps the numbers in one place:

  gates      the six verify gates (tools/verify.py char <id> --ssf2)
  paths      the C6 inputs whose path (where the move takes the fighter, frame by frame) strays
             past its tolerance from the source's (compare_paths over c6-paths-<id>.json)
  soak       script errors over ~3000 frames of play, and the frames the engine ran a second
             while the fighters played (the harness's own pace, so it reads the scripts' cost)
  hits       damage against the source's on every move that lands (hit_parity: the share, and
             the moves off it)
  states     rolls, dodges, grabs and throws: the length of each state against the source's
             (state_parity)
  projectiles what each special's projectiles do against the source's (proj_parity)
  freeze     the freeze a landed hit holds, against the source's (freeze_parity)
  timing     every animation at twice its source length (timing_parity)
  rollback   frames a local match that rolls back played again, and how many came out differently
             from their first playing (rollback_parity; the runtime's path only)
  size       the bytes of the fighter's scripts and of its packed .fra

usage: tools/tests/scoreboard.py --path=runtime|text [fighter ...] [--all]
       tools/tests/scoreboard.py --compare            (both paths' last runs side by side)
the path is chosen the way every tool here chooses it: PEPTIDE_RUNTIME set for the runtime's
path. results go to build/verify/scoreboard-<path>.json, a fighter at a time as each finishes, so
a long run can be read while it goes and a stopped one keeps what it measured.
"""
import glob, json, os, re, subprocess, sys, time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "build", "verify")
PY = sys.executable
GATES = ["C1", "C2", "C3", "C4", "C5", "C6"]


def run(args, env):
    t = time.time()
    p = subprocess.run(args, cwd=ROOT, env=env, capture_output=True, text=True)
    return p.stdout + p.stderr, time.time() - t


def load(path):
    try:
        return json.load(open(path, encoding="utf8"))
    except (OSError, ValueError):
        return None


def paths_off(cid):
    """the C6 inputs whose path strays past its tolerance, off the gate's own record"""
    sys.path.insert(0, os.path.join(ROOT, "tools"))
    import verify as V
    d = load(os.path.join(OUT, f"c6-paths-{cid}.json"))
    if not d:
        return None
    off = 0
    for inp, src in (d.get("source") or {}).items():
        port = (d.get("port") or {}).get(inp) or []
        c = V.compare_paths(V.move_path(src or [], 1, 1.0), V.move_path(port, 2, V.SIZE_SCALE))
        if c is not None and not c["within"]:
            off += 1
    return off


def roster():
    ssfs = os.environ.get("SSF2_SSFS_DIR", os.path.join(ROOT, "..", "ssf2-ssfs"))
    ids = sorted(os.path.basename(p)[:-4] for p in glob.glob(os.path.join(ssfs, "*.ssf")))
    return [i for i in ids if os.path.isdir(os.path.join(ROOT, "characters", i)) or not i.lower().startswith(("dat", "misc"))]


def measure(cid, env):
    row = {"fighter": cid}
    out, _ = run([PY, "tools/verify.py", "char", cid, "--ssf2"], env)
    gates = {}
    for g in GATES:
        m = re.search(rf"^\s+{g} \S+\s+(PASS|FAIL|SKIP)\s*(.*)$", out, re.M)
        gates[g] = (m.group(1), m.group(2)[:160]) if m else ("?", "")
    row["gates"] = gates
    row["gates_passed"] = sum(1 for g in GATES if gates[g][0] == "PASS")
    row["paths_off"] = paths_off(cid)

    out, secs = run([PY, "tools/tests/soak.py", cid], env)
    m = re.search(rf"== {re.escape(cid)}: (\d+) frames, (\d+) error line", out)
    frames = int(m.group(1)) if m else 0
    row["soak_errors"] = int(m.group(2)) if m else None
    row["soak_fps"] = round(frames / secs, 1) if m and secs > 0 else None

    run([PY, "tools/tests/hit_parity.py", cid], env)
    h = load(os.path.join(OUT, f"hits-{cid}.json")) or {}
    row["hit_share"] = h.get("damage_share")
    row["hit_problems"] = len(h.get("problems") or [])
    row["hit_unlanded"] = len(h.get("unlanded") or [])

    run([PY, "tools/tests/state_parity.py", cid], env)
    s = load(os.path.join(OUT, f"stateparity-{cid}.json")) or {}
    sc = s.get("scenarios") or {}
    row["states_checked"] = len(sc)
    # a scenario the source itself never reached (its grab closed on nothing) says nothing of the port
    row["states_off"] = sum(1 for v in sc.values() if isinstance(v, dict) and v.get("verdict")
                            and not v["verdict"].startswith("the source never"))

    out, _ = run([PY, "tools/tests/proj_parity.py", cid], env)
    # what each move's projectiles did, kept: the tool prints it and writes nothing else
    open(os.path.join(OUT, f"proj-{cid}.txt"), "w", encoding="utf8").write(out)
    m = re.search(r"(\d+) move\(s\) with projectiles, (\d+) differing", out)
    row["proj_moves"] = int(m.group(1)) if m else None
    row["proj_off"] = int(m.group(2)) if m else None

    run([PY, "tools/tests/freeze_parity.py", cid], env)
    f = load(os.path.join(OUT, f"freeze-{cid}.json")) or {}
    row["freeze_checked"] = len(f)
    row["freeze_off"] = sum(1 for v in f.values() if isinstance(v, dict) and v.get("source") and v.get("port")
                            and v["source"][1:] != v["port"][1:])

    out, _ = run([PY, "tools/tests/timing_parity.py", cid], env)
    m = re.search(r"TOTAL: (\d+) animations checked, (\d+) off", out)
    row["timing_checked"] = int(m.group(1)) if m else None
    row["timing_off"] = int(m.group(2)) if m else None

    if env.get("PEPTIDE_RUNTIME"):
        out, _ = run([PY, "tools/tests/rollback_parity.py", cid, "--frames=1200"], env)
        m = re.search(r"(\d+) replayed, (\d+) differ", out)
        row["replayed"] = int(m.group(1)) if m else None
        row["replay_off"] = int(m.group(2)) if m else None

    scripts = glob.glob(os.path.join(ROOT, "characters", cid, "library", "scripts", "**", "*.hx"), recursive=True)
    row["script_bytes"] = sum(os.path.getsize(p) for p in scripts)
    fra = glob.glob(os.path.join(ROOT, "characters", cid, "**", "*.fra"), recursive=True)
    row["fra_bytes"] = max((os.path.getsize(p) for p in fra), default=None)
    return row


COLUMNS = [("gates", "gates_passed"), ("paths off", "paths_off"), ("soak err", "soak_errors"), ("fps", "soak_fps"), ("hit share", "hit_share"),
           ("hits off", "hit_problems"), ("states off", "states_off"), ("proj off", "proj_off"),
           ("freeze off", "freeze_off"), ("timing off", "timing_off"), ("replays off", "replay_off"), ("script KB", "script_bytes")]


def cell(row, key):
    v = row.get(key) if row else None
    if v is None:
        return "-"
    if key == "script_bytes":
        return str(round(v / 1024))
    return str(v)


def compare():
    a = load(os.path.join(OUT, "scoreboard-text.json")) or {}
    b = load(os.path.join(OUT, "scoreboard-runtime.json")) or {}
    ids = sorted(set(a) | set(b))
    head = "| fighter | " + " | ".join(f"{n} (text / runtime)" for n, _ in COLUMNS) + " |"
    print(head)
    print("|" + "---|" * (len(COLUMNS) + 1))
    for cid in ids:
        print(f"| {cid} | " + " | ".join(f"{cell(a.get(cid), k)} / {cell(b.get(cid), k)}" for _, k in COLUMNS) + " |")
    for name, rows in (("text", a), ("runtime", b)):
        if rows:
            six = sum(1 for r in rows.values() if r.get("gates_passed") == 6)
            clean = sum(1 for r in rows.values() if r.get("soak_errors") == 0)
            print(f"\n{name}: {len(rows)} fighters, {six} pass all six gates, {clean} soak clean")


def main():
    if "--compare" in sys.argv:
        compare(); return
    path = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--path=")), None)
    if path not in ("runtime", "text"):
        print(__doc__); sys.exit(2)
    ids = [a for a in sys.argv[1:] if not a.startswith("--")]
    if "--all" in sys.argv or not ids:
        ids = roster()
    env = dict(os.environ)
    if path == "runtime":
        env["PEPTIDE_RUNTIME"] = "1"
    else:
        env.pop("PEPTIDE_RUNTIME", None)
    dest = os.path.join(OUT, f"scoreboard-{path}.json")
    board = load(dest) or {}
    for cid in ids:
        t = time.time()
        row = measure(cid, env)
        row["seconds"] = round(time.time() - t)
        board[cid] = row
        json.dump(board, open(dest, "w", encoding="utf8"), indent=1)
        print(f"{cid}: gates {row['gates_passed']}/6, paths {row.get('paths_off')} off, soak {row['soak_errors']} errors at {row['soak_fps']} fps, "
              f"hits {row['hit_share']} ({row['hit_problems']} off), states {row['states_off']} off, "
              f"proj {row['proj_off']} off, freeze {row['freeze_off']} off, timing {row['timing_off']} off, "
              f"replays {row.get('replay_off')} off of {row.get('replayed')} "
              f"[{row['seconds']}s]", flush=True)


if __name__ == "__main__":
    main()
