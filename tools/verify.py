#!/usr/bin/env python3
"""verify.py -- is this port RIGHT, and is it RUNNING? one answer per subject, from evidence.

every check this repo grew answered one question, and each new way a port could be wrong was
invisible to the check before it: a script the engine dropped whole reads as "no errors", a
stage spinning at 100% reads as "no errors", an element in a container the walk never opens
reads as "not there", a publish that timed out leaves the previous package installed and the
next check measures that. finding those one at a time, after calling the port done, is the
cycle this exists to end.

so the rule here is POSITIVE EVIDENCE, and it comes from two sources that cannot lie by
leaving something out:

  the plan        the converter writes `verify.json` beside every package it emits: what its
                  script makes, where, on what clock; which scripts the package carries; how
                  much art it ships. one statement of what the port is meant to be, written
                  from the same values that were emitted.
  the self-report the emitted script counts its own frames and keeps a registry of everything
                  it made. the live gates call it (`e __verifyDump()`, `e ...__verifyPulse()`)
                  inside the script's own scope and hold the answer to the plan. a script that
                  was dropped, hung, or never reached its spawn block cannot give the right
                  answer, which is the point.

gates run in order and a failed gate BLOCKS the rest: a later gate is never allowed to report
"fine" on top of a stale package or a wedged engine. every gate records what it saw, so a FAIL
says exactly which fact was missing.

stages    S1 convert   exit 0, plan written, scripts parse, no dangling art, within the publish budget
          S2 publish   export succeeded AND the installed package carries every planned script/entity
          S3 boot      match reaches STAND, engine not pinned, pulse advances, spawn block completed, no script errors
          S4 built     every planned object exists, at its planned position, on its planned clock
          S5 motion    every loop advanced by exactly the frames the stage ran (no wall clock, no aliasing)
          S6 stand     where each fighter comes to rest, engine against engine (--ssf2)
          S7 ssf2      the same stage in SSF2, compared object for object (--ssf2)
          S8 quiet     no script error in the whole match, after every gate above has driven it
characters C1 convert  exit 0, plan written, 30->60fps doubling per animation, hitbox parity, art resolves
          C2 publish   as above
          C3 boot      spawn reaches STAND, pulse advances, no script errors
          C4 physics   held-direction ground speed against the stats the fighter declares
          C5 moves     every move plays its own animation for its emitted length and returns to STAND
          C6 ssf2      the same inputs in SSF2, compared (--ssf2)

usage:
  tools/verify.py stage <id> [--ssf2] [--keep] [--speed=N]
  tools/verify.py char <id> [--ssf2] [--deep] [--speed=N] # N engine frames per tick (1..8)
                                                         # --deep drives the smashes, tilts and aerials too
  tools/verify.py sample [--ssf2]            # the ten stages + ten characters chosen for coverage
  tools/verify.py corpus [--stages] [--chars] [--ssf2]
reports land in build/verify/<kind>/<id>.json and build/verify/summary.md. exit 1 if anything failed.
"""
import atexit, glob, json, os, queue, re, shutil, struct, subprocess, sys, threading, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# the binary under test: a frozen copy through PEPTIDE_BIN keeps a long run on one build (the
# sweep script takes the same name), and a build of an older commit answers "did this change it"
BIN = os.environ.get("PEPTIDE_BIN") or os.path.join(ROOT, "build", "release", "peptide")
# a copy made by tools/freeze.sh carries what the binary reads from disk beside it; without
# these the copy would still read the working tree's mappings and engine scripts
if os.environ.get("PEPTIDE_BIN"):
    _frozen = os.path.dirname(os.path.abspath(BIN))
    if os.path.isdir(os.path.join(_frozen, "mappings")):
        os.environ.setdefault("PEPTIDE_MAPPINGS_DIR", os.path.join(_frozen, "mappings"))
    if os.path.isdir(os.path.join(_frozen, "assets")):
        os.environ.setdefault("PEPTIDE_ASSET_DIR", os.path.join(_frozen, "assets"))
SSFS = os.environ.get("SSF2_SSFS_DIR", os.path.join(ROOT, "..", "ssf2-ssfs"))
# the game's own data, when it is installed: what the SSF2 half of a comparison plays. the corpus
# is a sample that can be another build of the same content (mario's fireball clip is 18 frames
# in one and 23 in the other), and a port compared against a build it was not read from reports
# every difference between the two builds as a port bug. `PEPTIDE_VERIFY_SOURCE=corpus` keeps the
# corpus as the source.
SSF2_DATA = next((d for d in [
    os.environ.get("SSF2_DATA_DIR", ""),
    "/Applications/SSF2.app/Contents/Resources/data",
] if d and os.path.isdir(d)), None)


def source_ssf(cid, kind):
    """the file a subject is converted from: the installed game's own, else the corpus copy."""
    corpus = os.path.join(SSFS, "stages", f"{cid}.ssf") if kind == "stage" else os.path.join(SSFS, f"{cid}.ssf")
    if SSF2_DATA is None or os.environ.get("PEPTIDE_VERIFY_SOURCE") == "corpus":
        return corpus
    cache = os.path.join(ROOT, "build", "verify", "ssf2-data.json")
    table = None
    try:
        table = json.load(open(cache, encoding="utf8"))
        if table.get("dir") != SSF2_DATA:
            table = None
    except Exception:
        table = None
    if table is None:
        code, out, _ = run([BIN, "ssf2", "identify", SSF2_DATA], 900)
        table = {"dir": SSF2_DATA, "character": {}, "stage": {}}
        for line in out.splitlines():
            f = line.split()
            if len(f) >= 3 and f[0].endswith(".ssf") and f[2] in ("character", "stage"):
                table[f[2]][f[1]] = os.path.join(SSF2_DATA, f[0])
                if f[2] == "character" and len(f) >= 5 and f[3] == "chars:":
                    for sub in f[4].split(","):
                        table["character"].setdefault(sub, os.path.join(SSF2_DATA, f[0]))
        os.makedirs(os.path.dirname(cache), exist_ok=True)
        json.dump(table, open(cache, "w", encoding="utf8"), indent=1)
    return table.get(kind, {}).get(cid, corpus)
REPORTS = os.path.join(ROOT, "build", "verify")
SESSIONS = os.path.join(ROOT, "build", "verify", "sessions")
sys.path.insert(0, os.path.join(ROOT, "tools", "tests"))

# chosen for coverage, not convenience: hazards with their own classes, a phased backdrop, a
# nested-clip backdrop, moving plane elements, a ported class with weather, cross-script
# objects, a two-hundred-element stage, a thousand-asset stage, the plainest stage there is,
# and a day cycle.
SAMPLE_STAGES = ["bowserscastle", "castlesiege", "sectorz", "mushroomkingdom3", "finalvalley",
                 "fourside", "racetothefinish", "galaxytours", "battlefield", "clocktown"]
# the two reference fighters, a two-character package, projectile users, a charge special, a
# multi-jump, a fast faller, a heavy, and two with unusual specials.
SAMPLE_CHARS = ["sandbag", "mario", "zelda", "link", "samus", "kirby", "pikachu", "sonic",
                "donkeykong", "fox"]

# the publish step decodes every picture it imports, so what it can carry is measured in
# bytes of art more than in files: a fighter's thousand small cels publish in a minute, a
# stage's 1278 full-width frames (103MB) do not publish in ninety, and 1.6GB runs the publisher
# out of memory. the count is a backstop: kirby, with a hat and a pose for every fighter he
# copies and his colours, blends and filters drawn into his pictures, ships about 3100 small
# ones (21MB).
ASSET_BUDGET_IMAGES = 4000
ASSET_BUDGET_BYTES = 250 * 1024 * 1024
EXPORT_TIMEOUT_MS = int(os.environ.get("EXPORT_TIMEOUT", 2400000))
CONVERT_TIMEOUT_S = 2400

# the client's move vocabulary (src/interpreter.rs MOVES), minus the two that are not moves
MOVES = ["jab", "dash_attack", "tilt_forward", "tilt_up", "tilt_down", "strong_forward", "strong_up",
         "strong_down", "aerial_neutral", "aerial_forward", "aerial_back", "aerial_up", "aerial_down",
         "special_neutral", "special_side", "special_up", "special_down", "grab"]

# the engine's own crash marker, not the word: a trace row naming a `crash_bounce` frame is the
# fighter bouncing, and the moves gate reads every line of the run
ERROR_RE = re.compile(r"SCRIPTERR|\[ Error \]|Unknown variable|Invalid (?:call|field|access)|Null (?:Object|access)|\[crash\]", re.I)


def fraymakers_root():
    for c in [os.environ.get("FRAYMAKERS_DIR", ""),
              os.path.expanduser("~/Library/Application Support/Steam/steamapps/common/Fraymakers"),
              os.path.expanduser("~/.steam/steam/steamapps/common/Fraymakers"),
              os.path.expanduser("~/.local/share/Steam/steamapps/common/Fraymakers")]:
        if c and os.path.isdir(c):
            return c
    return None


def installed_fra(cid):
    r = fraymakers_root()
    return os.path.join(r, "custom", cid, f"{cid}.fra") if r else None


def read_fra(path):
    """the installed package: a length-prefixed json header ahead of the binary blobs."""
    with open(path, "rb") as f:
        head = f.read(4)
        n = struct.unpack(">I", head)[0]
        return json.loads(f.read(n))


def run(cmd, timeout, env=None, cwd=ROOT):
    t0 = time.time()
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd,
                           env=dict(os.environ, **(env or {})))
        return r.returncode, r.stdout + r.stderr, time.time() - t0
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        return -9, out + f"\n[timed out after {timeout}s]", time.time() - t0


# ----------------------------------------------------------------------------------------------
# the gate framework
# ----------------------------------------------------------------------------------------------
class Result:
    def __init__(self, name):
        self.name, self.status, self.why, self.evidence, self.seconds = name, "PASS", [], {}, 0.0

    def fail(self, why):
        self.status = "FAIL"
        self.why.append(why)

    def note(self, why):
        self.why.append(why)

    def skip(self, why):
        self.status = "SKIP"
        self.why.append(why)

    def d(self):
        return {"gate": self.name, "status": self.status, "why": self.why,
                "evidence": self.evidence, "seconds": round(self.seconds, 1)}


def run_gates_with_ahead(key, gates, ctx):
    """the subject's gates, with its convert and publish taken from the run done ahead of
    its turn when there is one (the first two gates, whose context carries into the rest)."""
    pre = AHEAD.pop(key, None)
    if pre is None:
        return run_gates(gates, ctx)
    pre_ctx, pre_results = pre
    for k, v in pre_ctx.items():
        ctx.setdefault(k, v)
    for r in pre_results:
        print(f"  {r.name:10} {r.status:5} {'; '.join(r.why)[:150]}", flush=True)
    blocked = next((r.name for r in pre_results if r.status == "FAIL" and blocks(r.name)), None)
    rest = []
    for name, fn in gates[len(pre_results):]:
        r = Result(name)
        t0 = time.time()
        if blocked:
            r.skip(f"blocked by {blocked}")
        else:
            try:
                fn(ctx, r)
            except Exception as e:
                r.fail(f"gate crashed: {type(e).__name__}: {e}")
        r.seconds = time.time() - t0
        rest.append(r)
        print(f"  {name:10} {r.status:5} {'; '.join(r.why)[:150]}", flush=True)
        if r.status == "FAIL" and not blocked and blocks(name):
            blocked = name
    return pre_results + rest


def run_gates(gates, ctx, quiet=False):
    results, blocked = [], None
    for name, fn in gates:
        r = Result(name)
        t0 = time.time()
        if blocked:
            r.skip(f"blocked by {blocked}")
        else:
            try:
                fn(ctx, r)
            except Exception as e:  # a gate that crashes has proved nothing
                r.fail(f"gate crashed: {type(e).__name__}: {e}")
        r.seconds = time.time() - t0
        results.append(r)
        if not quiet:
            print(f"  {name:10} {r.status:5} {'; '.join(r.why)[:150]}", flush=True)
        if r.status == "FAIL" and not blocked and blocks(name):
            blocked = name
    return results


def blocks(name):
    """whether a gate's failure leaves nothing for the gates after it to read: a package that did
    not convert, publish or boot. the rest are each their own reading of a running match, and one
    of them failing (a source whose fighters a moving stage carries off) must not hide the others"""
    return name.split()[-1] in ("convert", "publish", "boot")


# ----------------------------------------------------------------------------------------------
# a live engine, driven through the session and read through its log
# ----------------------------------------------------------------------------------------------
# the engines this run has up, one per kind: a subject reuses the one already open by
# starting its match in it, and the app is launched only when none is up (or the one up
# cannot start the match, in which case it is replaced)
LIVE = {}


def close_live():
    """end the engines this run holds (only on `--shutdown`; they stay up for the next run)"""
    for eng in list(LIVE.values()):
        eng.close(final=True)


class Engine:
    """a session and the game it drives; a subject either adopts the one already open (a new
    match in it) or boots one. what it reads of the log is scoped to its own match, so a
    subject never reports the errors of the one before it."""
    kind = "fraymakers"

    def __init__(self, tag):
        # one directory per engine kind: the session in it outlives a run, and the next run
        # drives the game already open there rather than launching another
        self.dir = os.path.join(SESSIONS, self.kind)
        self.tag = tag
        self.log_path = os.path.join(self.dir, "out.log")
        self.proc = None
        self.mark0 = 0
        self.roster = None  # (chars, stage, assist) of the match this engine is in

    def pairing(self):
        """the session daemon already running in this engine's directory, if its game is up
        and its history is known (what it has loaded, kept beside it): its pid."""
        if not os.path.exists(self.loaded_path()):
            return None
        try:
            kv = dict(l.split("=", 1) for l in open(os.path.join(self.dir, "meta"), encoding="utf8").read().splitlines() if "=" in l)
            pid = int(kv.get("pid", "0"))
            if pid <= 0:
                return None
            os.kill(pid, 0)
            ep = kv.get("engine_pid", "").strip()
            if ep:
                os.kill(int(ep), 0)
            return pid
        except (FileNotFoundError, ValueError, ProcessLookupError, PermissionError):
            return None

    def adopt_pairing(self, pid):
        """drive the session a previous run left up: its log from here on, its process not
        ours to end."""
        if self.kind == "ssf2":
            self.log_path = os.path.join(self.dir, "boot.log")
            self.feed_path = os.path.join(self.dir, "out.log")
        self.proc = None
        self.daemon_pid = pid
        # the log carries every earlier run; what this run reads starts here
        self.mark0 = len(self.log())
        self._feed = None
        self.feed_start()
        LIVE[self.kind] = self
        self.roster = None

    def crashed(self):
        """the engine died in this subject's match: its crash line is in the log, or the
        process that carried it is gone. every answer a gate waits for after that is dead air."""
        return "[crash]" in self.log()[self.mark0:] or not self.alive()

    def alive(self):
        if self.proc is not None:
            return self.proc.poll() is None
        pid = getattr(self, "daemon_pid", None)
        if pid:
            try:
                os.kill(pid, 0)
                return True
            except (ProcessLookupError, PermissionError):
                return False
        return False

    def adopt(self, live):
        """take over the session `live` holds: its directory, process and feed."""
        self.dir, self.log_path, self.proc = live.dir, live.log_path, live.proc
        self.daemon_pid = getattr(live, "daemon_pid", None)
        self.feed_path = getattr(live, "feed_path", None)
        self._feed, self._q = getattr(live, "_feed", None), getattr(live, "_q", None)
        self.speed = getattr(live, "speed", 1)
        self.roster = live.roster
        live._feed, live.proc = None, None
        LIVE[self.kind] = self

    def wanted(self, args):
        """the match `args` ask for, over what the engine is in: (chars, stage, assist)."""
        chars, stage, assist = self.roster or (None, None, None)
        for i, a in enumerate(args):
            if a == "--char" and i + 1 < len(args):
                c = args[i + 1]
                chars = c if "," in c else f"{c},{c}"
            if a == "--stage" and i + 1 < len(args):
                stage = args[i + 1]
        return chars, stage, assist

    def spawn_cmd(self, args):
        chars, stage, assist = self.wanted(args)
        return " ".join(x for x in ("spawn", chars, stage, assist) if x), (chars, stage, assist)

    # ---- what this engine process has loaded, kept beside its session ----
    def loaded_path(self):
        return os.path.join(self.dir, "loaded.txt")

    def loaded(self):
        try:
            return set(open(self.loaded_path(), encoding="utf8").read().split())
        except FileNotFoundError:
            return set()

    def note_loaded(self, ids):
        with open(self.loaded_path(), "a", encoding="utf8") as f:
            f.write(" ".join(ids) + "\n")

    def relaunch(self, args, timeout=180):
        """a new match in the running engine; seconds it took, or None when it never came
        (or when the engine would bring the content up stale)."""
        t0 = time.time()
        cmd, roster = self.spawn_cmd(args)
        if not roster[0]:
            return None
        mark = self.tell(cmd)
        if self.reply(mark, "LAUNCHED", timeout) is None or not self.alive():
            return None
        # what this subject reads of the log starts at its match: the switch between two
        # matches is the engine's own business, not the new fighter's
        self.mark0 = len(self.log())
        # the NEW match's own intro and stand: the last match's are in the log too
        while "ANIM:INTRO" not in self.log()[mark:] and "ANIM:STAND" not in self.log()[mark:]:
            if time.time() - t0 > timeout or not self.alive():
                return None
            self.next_line(1.0)
        if self.await_pose(("stand", "idle"), timeout=60.0) >= 60.0:
            return None
        self.roster = roster
        self.note_loaded(roster[0].split(","))
        return time.time() - t0

    def boot(self, args, wait_for=("engine READY", "LAUNCHED"), timeout=180):
        live = LIVE.get(self.kind)
        if live is None:
            pid = self.pairing()
            if pid:
                self.adopt_pairing(pid)
                live = self
        if live is not None and live.alive():
            if live is not self:
                self.adopt(live)
            took = self.relaunch(args)
            if took is not None:
                return took
            # the engine could not start this match: it is replaced by a fresh one
            self.close(final=True)
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir, exist_ok=True)
        self.mark0 = 0
        self.daemon_pid = None
        subprocess.run(["pkill", "-x", "hl"], capture_output=True)
        for _ in range(100):
            if subprocess.run(["pgrep", "-x", "hl"], capture_output=True).returncode != 0:
                break
            time.sleep(0.05)
        self._feed = None
        self.feed_start()
        self.proc = subprocess.Popen([BIN, "session", *args, "--dir", self.dir],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        took = self.wait(wait_for, timeout)
        if took is not None:
            LIVE[self.kind] = self
            m = re.search(r"auto-launching \(spawn ([^ )]+) ?([^ )]*) ?([^ )]*)\)", self.log())
            self.roster = (m.group(1), m.group(2) or None, m.group(3) or None) if m else self.wanted(args)
            self.note_loaded((self.roster[0] or "").split(","))
        return took

    def log(self):
        try:
            return open(self.log_path, encoding="utf8", errors="replace").read()
        except FileNotFoundError:
            return ""

    def wait(self, needles, timeout):
        """block until every needle has appeared in the log since this subject's match began,
        checked as each line arrives on the feed; the seconds it took, or None."""
        t0 = time.time()
        while True:
            text = self.log()[self.mark0:]
            if all(n in text for n in needles):
                return time.time() - t0
            if self.proc and self.proc.poll() is not None:
                return None
            left = timeout - (time.time() - t0)
            if left <= 0:
                return None
            self.next_line(min(left, 1.0))

    def tell(self, cmd):
        """queue one command: a line appended to the session's control file, which is all
        `peptide tell` does, without spawning a process for it."""
        # this engine takes a seq's inputs one per TICK, not one per frame: at eight frames a
        # tick a two-frame tap is held for sixteen and a walk of sixteen for over a hundred. a
        # seq plays at the engine's own rate, and the rate comes back once its steps are taken
        if cmd.startswith("seq ") and type(self) is Engine and getattr(self, "speed", 1) > 1:
            self._seq_speed = self.speed
            self.reply(self._raw_tell("speed 1"), "", 10)
        return self._raw_tell(cmd)

    def _raw_tell(self, cmd):
        mark = len(self.log())
        with open(os.path.join(self.dir, "control"), "a", encoding="utf8") as f:
            f.write(cmd + "\n")
        return mark

    def tell_batch(self, cmds):
        """several commands queued at once: the session drains them one per frame, so what must
        happen within a frame or two of each other is sent this way rather than as separate
        round trips (each of which is a dozen frames)."""
        mark = len(self.log())
        with open(os.path.join(self.dir, "control"), "a", encoding="utf8") as f:
            f.write("\n".join(cmds) + "\n")
        return mark

    @staticmethod
    def _strip_reply(s, prefix):
        s = s.strip()
        for lead in ("<< " + prefix, prefix):
            if s.startswith(lead):
                return s[len(lead):].strip()
        return None

    # lines that mean the answer this call is waiting for will NEVER come, so waiting the full
    # timeout is dead air: a script error on the very command whose reply we want, or the engine
    # dying. Detected AFTER the command's mark (commands are serialised), so it belongs to this
    # command's turn. This is what stops a probe on a stage whose source throws from eating 10 s.
    _FAILFAST = re.compile(r"SCRIPTERR|Script (?:Interpret|Parse) Error|Unknown variable|\[crash\]|"
                           r"Uncaught exception|Called from|hl_error")

    def reply(self, mark, prefix, timeout=10.0, failfast=True):
        """the first line after `mark` that the engine sent back with this prefix: what already
        arrived is read off the log, the rest waited for on the feed, line by line.

        `failfast` (default): the moment the engine reports an error on this command's turn (a
        SCRIPTERR, an unknown variable, a crash) the answer can never arrive, so return at once
        instead of waiting out `timeout` -- a read that cannot be answered costs ~0.1 s, not 10."""
        self.drain()
        for line in self.log()[mark:].splitlines():
            got = self._strip_reply(line, prefix)
            if got is not None:
                return got
            if failfast and self._FAILFAST.search(line):
                self.last_error = line.strip()
                return None
        t0 = time.time()
        while True:
            left = timeout - (time.time() - t0)
            if left <= 0:
                return None
            line = self.next_line(left)
            if line is None:
                return None
            got = self._strip_reply(line, prefix)
            if got is not None:
                return got
            if failfast and self._FAILFAST.search(line):
                self.last_error = line.strip()
                return None

    # every display object the walk lists, on either engine. fraymakers prints `pkg.Class @(x,y)
    # frame=a/b 'anim'`; the source prints `[object Class] "name" @(x,y) WxH [HIDDEN] frame=a/b
    # 'label' abs=(ax,ay)`. the class is whatever the engine calls it (data read at run time, never
    # a name written here); `abs` is the source's world bounding-box corner, the number its local
    # @(x,y) is not.
    _OBJ_FM = re.compile(r"([\w.]+)\.(\w+) @\((-?[\d.]+),(-?[\d.]+)\)(?: frame=(-?\d+)/(-?\d+))?(?: '([^']*)')?")
    _OBJ_SSF2 = re.compile(r"\[object (\S+)\] \"([^\"]*)\" @\((-?[\d.]+),(-?[\d.]+)\)(?: (\d+)x(\d+))?( HIDDEN)?"
                           r"(?: frame=(-?\d+)/(-?\d+))?(?: '([^']*)')?(?: abs=\((-?[\d.]+),(-?[\d.]+)\))?")

    def _tree_text(self, timeout=10.0):
        """the raw object-walk output. Fraymakers answers a tagged eval right after the walk, so
        the walk is everything before that reply."""
        self.n = getattr(self, "n", 0) + 1
        tag = f"v{self.n}:"
        mark = self.tell_batch(["tree", f'e "{tag}" + 1'])
        if self.reply(mark, "E:" + tag, timeout) is None:
            return ""
        return self.log()[mark:]

    def objects(self, timeout=10.0):
        """every display object on screen, read off the object walk: one dict per object with
        cls, name, x, y (world where the engine gives it), w, h, hidden, anim, frame, total. what
        a move SPAWNS is the difference between this and a snapshot taken before the move; what it
        does to them is how the same objects read frame to frame. the walk lists an object under
        every container it hangs from, so entries collapse on (cls, name, position)."""
        text = self._tree_text(timeout)
        out, seen = [], set()
        for m in self._OBJ_SSF2.finditer(text):
            cls, name = m.group(1), m.group(2)
            # the world corner when the walk gives one, else the local position
            x = float(m.group(11)) if m.group(11) else float(m.group(3))
            y = float(m.group(12)) if m.group(12) else float(m.group(4))
            key = (cls, name)
            if key in seen:
                continue
            seen.add(key)
            out.append({"cls": cls, "name": name, "x": x, "y": y,
                        "w": int(m.group(5)) if m.group(5) else None, "h": int(m.group(6)) if m.group(6) else None,
                        "hidden": bool(m.group(7)), "frame": int(m.group(8)) if m.group(8) else None,
                        "total": int(m.group(9)) if m.group(9) else None, "anim": m.group(10)})
        if out:
            return out
        for m in self._OBJ_FM.finditer(text):
            cls = m.group(2)
            x, y = float(m.group(3)), float(m.group(4))
            key = (cls, round(x, 1), round(y, 1), m.group(7))
            if key in seen:
                continue
            seen.add(key)
            out.append({"cls": cls, "name": None, "x": x, "y": y, "w": None, "h": None, "hidden": False,
                        "frame": int(m.group(5)) if m.group(5) else None,
                        "total": int(m.group(6)) if m.group(6) else None, "anim": m.group(7)})
        return out

    _PROJ = re.compile(r"FraymakersProjectile @\((-?[\d.]+),(-?[\d.]+)\)(?: frame=(\d+)/(\d+) '([^']*)')?")

    def projectiles(self, timeout=10.0):
        """where every live projectile is, read off the engine's own object walk (`tree`) rather
        than an hscript `getX()` on the handle: a projectile api object answers that read with a
        field error, or takes the engine down (signal 11), while the walk reads the same position
        from bytecode and never throws. one dict per projectile: x, y, and when the walk carries
        it, the animation and its frame. the walk has no end marker, so a tagged eval is queued
        right behind it and the walk is what arrived before the eval's reply.

        the walk reads a display container, which holds a projectile a beat past its death (killed
        projectiles linger there while the engine's own `getProjectiles()` list has already dropped
        them), so the walk over-reports and, worse, goes stale: a shot that has despawned still
        reads as present. the tagged eval carries the AUTHORITATIVE live count, and the result is
        trimmed to it, keeping the entries that carry an animation (an active shot renders a frame)
        over the anim-less ghosts. this is what makes a despawn observable off the walk."""
        self.n = getattr(self, "n", 0) + 1
        tag = f"v{self.n}:"
        mark = self.tell_batch(["tree", f'e "{tag}" + match.getProjectiles().length'])
        got = self.reply(mark, "E:" + tag, timeout)
        if got is None:
            return []
        try:
            n_live = int(float(got))
        except (TypeError, ValueError):
            n_live = None
        # the walk lists an object under every container it hangs from (once bare, once with its
        # animation), so entries collapse by position, keeping the one that carries the animation
        by_pos = {}
        for m in self._PROJ.finditer(self.log()[mark:]):
            x, y = float(m.group(1)), float(m.group(2))
            key = (round(x, 1), round(y, 1))
            ent = {"x": x, "y": y, "anim": m.group(5),
                   "frame": int(m.group(3)) if m.group(3) else None,
                   "total": int(m.group(4)) if m.group(4) else None}
            if key not in by_pos or (ent["anim"] and not by_pos[key]["anim"]):
                by_pos[key] = ent
        ents = list(by_pos.values())
        # trim stale display-container ghosts down to the engine's own live count: an active shot
        # renders an animation, so an anim-bearing entry outranks an anim-less one for a kept slot
        if n_live is not None and n_live < len(ents):
            ents.sort(key=lambda e: e["anim"] is None)
            ents = ents[:n_live]
        return ents

    def n_projectiles(self, timeout=10.0):
        """the engine's OWN count of live projectiles (`match.getProjectiles().length`): authoritative
        and crash-safe (a length read touches no projectile field), unlike the display-walk count
        the tree gives, which lingers a beat past a despawn. use this to decide whether a shot is
        still alive; use `projectiles()` for where the live ones are."""
        v = self.eval("match.getProjectiles().length", timeout)
        try:
            return int(float(v))
        except (TypeError, ValueError):
            return None

    def eval(self, expr, timeout=10.0):
        """the value of an expression in the running script's scope.

        every reply is tagged with its own number so it can never be paired with the wrong
        question: replies arrive on their own time, and the first one after a command was sent
        was, once in a while, the answer to the command before it."""
        self.n = getattr(self, "n", 0) + 1
        tag = f"v{self.n}:"
        mark = self.tell(f'e "{tag}" + ({expr})')
        got = self.reply(mark, "E:" + tag, timeout)
        return got

    def cpu(self):
        r = subprocess.run(["ps", "-A", "-o", "%cpu=,comm="], capture_output=True, text=True)
        for line in r.stdout.splitlines():
            parts = line.split(None, 1)
            if len(parts) == 2 and os.path.basename(parts[1].strip()) == "hl":
                try:
                    return float(parts[0])
                except ValueError:
                    pass
        return None

    # ── the match clock ───────────────────────────────────────
    # a moving platform is wherever its cycle has reached, and the two engines reach a moment
    # of the match at different wall times (each boots and settles on its own). fraymakers
    # answers its match frame count exactly; the source engine has no such counter to ask, so
    # its frames are accrued: wall time at its native rate, at whatever speed it was run, from
    # the moment its match spawned. good to a few frames over a gate's run.
    native_fps = 60

    def _accrue(self):
        now = time.time()
        since = getattr(self, "speed_since", None) or getattr(self, "spawned_at", None)
        if since is not None:
            self.frames_acc = getattr(self, "frames_acc", 0.0) + (now - since) * self.native_fps * getattr(self, "speed", 1)
        self.speed_since = now

    def clock(self):
        """the frames the session has seen so far, off the engine's own pushed frame clock
        (`clock` answers `CLOCK:N frames` on both engines)"""
        r = self.reply(self.tell("clock"), "CLOCK:", 10)
        m = re.match(r"\s*(\d+)", r or "")
        return int(m.group(1)) if m else None

    def frames_since_spawn(self):
        """the match's frame count, in this engine's own frames: the session's frame clock less
        its reading when the match came up, or, for a session that cannot answer it, an
        estimate from the time since the spawn at the rate the engine was run at"""
        c0 = getattr(self, "clock0", None)
        if c0 is not None:
            c = self.clock()
            if c is not None:
                return max(0, c - c0)
        self._accrue()
        return int(getattr(self, "frames_acc", 0.0))

    def mark_spawn(self):
        """the match is up: what the frame clock reads now is its frame 0"""
        self.spawned_at = time.time()
        self.speed_since = self.spawned_at
        self.frames_acc = 0.0
        self.clock0 = self.clock()

    def faster(self, n):
        """n engine frames per tick. every gate paces on the engine's frame count, so the
        measurements are the same; they arrive sooner."""
        self._accrue()
        self.speed = max(1, n or 1)
        if self.speed > 1:
            # each engine answers in its own words; the wait is for the answer itself
            self.reply(self.tell(f"speed {self.speed}"), "", 10)

    # ── the live feed ──────────────────────────────────────────────────────────────────────
    # the session holds the engine's socket and appends every event it pushes (ANIM:, FRAME:,
    # the replies) to its log the moment it arrives. a follower on that file wakes on each line,
    # so a wait here returns on the event itself and never sleeps for a guess at its timing.
    def feed_start(self):
        if getattr(self, "_feed", None) is not None:
            return
        path = getattr(self, "feed_path", None) or self.log_path
        open(path, "a", encoding="utf8").close()
        self._q = queue.Queue()
        self._feed = subprocess.Popen(["tail", "-n", "0", "-F", path],
                                      stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        def pump(proc, q):
            for line in proc.stdout:
                q.put(line.rstrip("\n"))
        threading.Thread(target=pump, args=(self._feed, self._q), daemon=True).start()

    def feed_stop(self):
        f = getattr(self, "_feed", None)
        if f is not None:
            try:
                f.terminate()
            except Exception:
                pass
            self._feed = None

    def next_line(self, timeout):
        """the next line the engine wrote, or None when `timeout` seconds pass without one."""
        self.feed_start()
        try:
            return self._q.get(timeout=max(0.0, timeout))
        except queue.Empty:
            return None

    def drain(self):
        """every line already waiting on the feed, without blocking."""
        self.feed_start()
        out = []
        while True:
            try:
                out.append(self._q.get_nowait())
            except queue.Empty:
                return out

    # the poses a move comes back to: seen after a move started, the move is over
    TERMINAL = ("stand", "idle", "fall", "helpless", "dead", "land", "land_light", "land_heavy",
                "heavyland", "revival", "fall_special", "fall_loop", "fall_in")

    def is_terminal(self, a):
        """a pose a move comes back to, however the engine spells the one it drifted into (a
        bored idle a few seconds in is still the fighter waiting)"""
        a = a.lower()
        return a in self.TERMINAL or a.startswith(("stand", "idle"))

    def anims_since(self, mark):
        """the animation names the engine announced since `mark`, in order."""
        return [m.group(1).lower() for m in re.finditer(r"ANIM:([A-Za-z0-9_]+)", self.log()[mark:])]

    @staticmethod
    def _anim_of(line):
        m = re.search(r"ANIM:([A-Za-z0-9_]+)", line or "")
        return m.group(1).lower() if m else None

    def settle(self, mark, timeout=8.0, quiet=None):
        """block until a move that started after `mark` has come back to a terminal pose, the
        moment the engine announces it; or until a short grace passes with no move at all (the
        input may have done nothing). seconds spent."""
        t0 = time.time()
        # the feed first, then the log: a line the feed already carries is in the log, and a
        # line written after this read will still arrive on the feed
        self.drain()
        started = False
        for a in self.anims_since(mark):
            if not self.is_terminal(a):
                started = True
            elif started:
                return time.time() - t0
        if not started:
            # whether the input started anything is known a few engine frames on: asked of
            # the engine's own clock, then the announcements since are read once more
            self.tick(8)
            for a in self.anims_since(mark):
                if not self.is_terminal(a):
                    started = True
                elif started:
                    return time.time() - t0
            if not started:
                return time.time() - t0
        while True:
            left = timeout - (time.time() - t0)
            if left <= 0:
                return time.time() - t0
            line = self.next_line(left)
            if line is None:
                continue
            a = self._anim_of(line)
            if a is None:
                continue
            if not self.is_terminal(a):
                started = True
            elif started:
                return time.time() - t0

    def await_seq(self, mark, steps, timeout=6.0):
        """block until the engine has taken every step of the seq sent at `mark`: it takes one
        per frame and acknowledges each, so once the last acknowledgement is in, whatever the
        input started has already been announced. seconds spent."""
        t0 = time.time()
        want = steps + 1  # the steps, then the release
        try:
            while True:
                self.drain()
                if self.log()[mark:].count("I:OK") >= want:
                    return time.time() - t0
                left = timeout - (time.time() - t0)
                if left <= 0:
                    return time.time() - t0
                self.next_line(left)
        finally:
            back = getattr(self, "_seq_speed", None)
            if back:
                self._seq_speed = None
                self.reply(self._raw_tell(f"speed {back}"), "", 10)

    def await_pose(self, poses=("stand", "idle"), timeout=6.0):
        """block until the fighter's latest announced animation is one of `poses`: a fighter that
        died on the last input is reviving, and an input sent into that is lost. seconds spent."""
        t0 = time.time()
        self.drain()
        a = self.anims_since(self.mark0)
        if a and a[-1] in poses:
            return 0.0
        while True:
            left = timeout - (time.time() - t0)
            if left <= 0:
                return time.time() - t0
            line = self.next_line(left)
            if line is None:
                return time.time() - t0
            if self._anim_of(line) in poses:
                return time.time() - t0

    def read_trace(self, mark, timeout=20):
        """the trace the engine answers after `mark`: its header says how many lines follow, and
        exactly those are waited for."""
        head = self.reply(mark, "TRACE", timeout)
        if head is None:
            return ""
        m = re.search(r"(\d+) animations", head) or re.search(r"RAW:(\d+) frames", "TRACE" + head)
        n = int(m.group(1)) if m else 0
        # lines that already landed in the log behind the header, then the rest off the feed
        text = self.log()[mark:]
        have = text.split("TRACE", 1)[-1].count("\n") - 1
        got = 0
        while have + got < n:
            if self.next_line(timeout) is None:
                break
            got += 1
        # the feed IS the log: a line waited for above is in it now, so the log alone holds every
        # row once (it was handed back with those lines added again, and a late row counted twice)
        return self.log()[mark:]

    def read_tree(self, mark=None, timeout=8.0, quiet=0.4):
        """the full object tree in one round trip, without a fixed poll: the walk arrives as a
        burst of lines and then stops, so read until the feed has been silent for `quiet`
        seconds (the burst is over) and return at once -- ~0.4 s after the last line, not the
        whole `timeout`. Fails fast if the tree cannot be built (an error on this turn)."""
        if mark is None:
            mark = self.tell("tree")
        # wait for the walk to actually start (its header), failing fast on an error
        if self.reply(mark, "  [tickables] n=", timeout) is None:
            return self.log()[mark:]
        t0 = time.time()
        while time.time() - t0 < timeout:
            if self.next_line(quiet) is None:
                break                          # silent for `quiet`s: the burst is done
        return self.log()[mark:]

    def catch(self, pred, budget=240, timeout=None):
        """run the engine forward up to `budget` frames and return the FIRST frame that satisfies
        `pred`, in ONE round trip -- not one probe per frame. the engine already pushes a per-frame
        record (`<label>|<frame>|<x>|<y>` for the driven fighter); this arms that recorder, ticks
        the whole budget, reads the window back once, and scans it host-side for the frame you were
        waiting for. so catching "the frame the animation reaches label X" or "the frame x crosses
        0" costs the time to play the budget plus one trace read, and a condition that never comes
        true costs exactly the same -- it returns None at the end of the budget instead of eating a
        wall-clock timeout waiting for a signal that was never going to arrive.

        `pred(row)` is called with a dict {i, label, frame, x, y}: `i` is the frame's index in the
        window (0-based, the ordinate to pace on), `label` the animation lowercased, `frame` its
        own frame counter, `x`/`y` the driven fighter's position. returns the first matching row,
        or None. a plain string is accepted for `pred` as shorthand for "label is exactly this"
        (exact, so `catch("jump")` catches jump and not jumpsquat -- pass a lambda with
        `.startswith`/`in` when you want a looser match)."""
        if isinstance(pred, str):
            needle = pred.lower()
            pred = lambda row: row["label"] == needle
        self.tell("record")                       # clear the window
        self.tick(budget, timeout=timeout)        # play it out on the engine's own clock
        for row in self._frame_window():
            if pred(row):
                return row
        return None

    def _frame_window(self):
        """the per-frame record the engine holds, parsed in one round trip: one dict per frame
        {i, label, frame, x, y}. shared by `catch` and any caller that wants the whole window."""
        mark = self.tell("trace raw")
        head = self.reply(mark, "TRACERAW:", 20)
        if head is None:
            return []
        m = re.search(r"(\d+) frames", head)
        want = int(m.group(1)) if m else 0
        def have():
            return sum(1 for l in self.log()[mark:].splitlines() if re.match(r"(?:<< )?\s*\d+\t", l))
        t0 = time.time()
        while have() < want and time.time() - t0 < 10:
            if self.next_line(1.0) is None:
                break
        rows, i = [], 0
        for line in self.log()[mark:].splitlines():
            m = re.match(r"(?:<< )?\s*\d+\t(.*)", line)
            if not m:
                continue
            # the record is `<label>|<LABEL>|<x>|<y>|<frame>`: x/y at 2/3 (same columns the
            # RLE parser reads), the animation's own frame counter last. read frame defensively
            # (the field's position can differ across engines), falling back to the window index.
            f = m.group(1).split("|")
            if len(f) < 4:
                continue
            try:
                x, y = float(f[2]), float(f[3])
            except ValueError:
                continue
            frame = i
            for g in f[4:]:
                try:
                    frame = int(float(g)); break
                except ValueError:
                    pass
            rows.append({"i": i, "label": f[0].strip().lower(), "frame": frame, "x": x, "y": y})
            i += 1
        return rows

    def tick(self, frames, timeout=None):
        """block until `frames` engine frames have passed, on the engine's own clock: the
        session answers `tick N` the moment the Nth pushed frame arrives. seconds spent."""
        t0 = time.time()
        if timeout is None:
            timeout = 2.0 + frames / 20.0
        # not failfast: a script's error mid-move (a projectile's 20 s DESTROYING warning) does not
        # stop the clock, and a tick that returned on it let the next command go out while the
        # engine was still ticking, its answer then cut short by the same warning's second copy
        self.reply(self.tell(f"tick {int(frames)}"), "TICK:", timeout, failfast=False)
        return time.time() - t0

    def nap(self, seconds):
        """a wait that used to be sized in seconds: now the same span in engine frames, on
        the engine's clock (sixty a second at its own rate, however fast it is run)."""
        return self.tick(max(1, round(seconds * 60)))

    def crash_reason(self):
        """what the session said when the engine died, if it did: the exception and where."""
        lines = [l.strip() for l in self.log().splitlines() if "[crash]" in l and ("exception" in l.lower() or "site" in l.lower())]
        why = (": " + "; ".join(l.replace("[crash]", "").strip() for l in lines[:2])) if lines else ""
        # the engine is patched to boot without its storefront; a boot that still died asking for
        # it was run with PEPTIDE_NEED_STEAM set, or against an engine the patch could not reach
        if "Steam" in why:
            try:
                wh = open(os.path.expanduser("~/Library/Application Support/Steam/logs/webhelper.txt"),
                          encoding="utf8", errors="replace").read()[-20000:]
                if "'Refresh Si" in wh or "-'Shutdown'" in wh:
                    why += " (Steam is asking its user to sign in again and quits when a game attaches; the engine should not have needed it: check the patch log)"
            except OSError:
                pass
        return why

    def errors(self):
        """the engine's script errors in this subject's own match.

        an `ANIM:` line is the driven fighter's per-frame animation announcement, not a script
        error: a stage whose hazard has an animation label like `CRASH_BOUNCE` (bowser's castle's
        thwomp) would otherwise trip the `crash` needle every time it plays that anim."""
        return [l.strip()[:160] for l in self.log()[self.mark0:].splitlines() if ERROR_RE.search(l)
                and "Unknown language" not in l
                and "ANIM:" not in l
                # the engine's own complaint about its own stock projectiles: a stock fighter
                # raises it in a match with no converted content in it
                and "DESTROYING state" not in l
                # an error in an expression the HARNESS evaluated (a probe, a self-check against a
                # script that predates it) is the harness's, not the subject's: the gates that
                # asked read its answer themselves
                and "(origin: hscript)" not in l]

    def close(self, final=False):
        """the engine stays up for the next subject; `final` shuts it down."""
        if not final:
            return
        if LIVE.get(self.kind) is self:
            del LIVE[self.kind]
        self.feed_stop()
        try:
            self.tell("exit")
            if self.proc:
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
            else:
                for _ in range(100):
                    if not self.alive():
                        break
                    time.sleep(0.05)
        finally:
            if self.proc:
                self.proc.terminate()
            subprocess.run(["pkill", "-x", "hl"], capture_output=True)


class Ssf2Engine(Engine):
    native_fps = 30
    """the other engine, driven through the same vocabulary. its session writes the replies to
    its own log the same way, so the reads are the Engine's."""

    kind = "ssf2"

    def relaunch(self, args, timeout=180):
        t0 = time.time()
        cmd, roster = self.spawn_cmd(args)
        if not roster[0]:
            return None
        mark = self.tell(cmd)
        if self.reply(mark, "spawn", timeout) is None or not self.alive():
            return None
        self.mark0 = len(self.log())
        while "ANIM:" not in self.log()[mark:]:
            if time.time() - t0 > timeout or not self.alive():
                return None
            self.next_line(1.0)
        self.roster = roster
        self.mark_spawn()
        return time.time() - t0

    def boot(self, args, wait_for=("peptide ssf2 tell",), timeout=180):
        live = LIVE.get(self.kind)
        if live is None:
            pid = self.pairing()
            if pid:
                self.adopt_pairing(pid)
                live = self
        if live is not None and live.alive():
            if live is not self:
                self.adopt(live)
            took = self.relaunch(args)
            if took is not None:
                return took
            self.close(final=True)
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir, exist_ok=True)
        self.mark0 = 0
        self.daemon_pid = None
        subprocess.run(["pkill", "-f", "ssf2 session --"], capture_output=True)
        subprocess.run(["pkill", "-f", "SSF2-patched"], capture_output=True)
        for _ in range(100):
            if subprocess.run(["pgrep", "-x", "SSF2-patched"], capture_output=True).returncode != 0:
                break
            time.sleep(0.05)
        # SSF2 decrypts its assets on demand and a cold boot occasionally never reaches the
        # ready line within the window (a stale patched process, a slow decrypt). that is the
        # engine being flaky, not the stage being wrong, so a boot that times out is tried once
        # more from a clean kill before it is called a failure.
        took = None
        for attempt in range(2):
            if attempt > 0:
                subprocess.run(["pkill", "-f", "ssf2 session --"], capture_output=True)
                subprocess.run(["pkill", "-f", "SSF2-patched"], capture_output=True)
                for _ in range(100):
                    if subprocess.run(["pgrep", "-x", "SSF2-patched"], capture_output=True).returncode != 0:
                        break
                    time.sleep(0.05)
                self.mark0 = 0
            self.log_path = os.path.join(self.dir, "boot.log")
            # the events and replies land in out.log; the boot in boot.log
            self.feed_path = os.path.join(self.dir, "out.log")
            self._feed = None
            self.feed_start()
            self.proc = subprocess.Popen([BIN, "ssf2", "session", *args, "--dir", self.dir],
                                         stdout=open(self.log_path, "w"), stderr=subprocess.STDOUT)
            took = self.wait(wait_for, timeout)
            if took is not None:
                break
            self.close(final=True)
        if took is not None:
            LIVE[self.kind] = self
            m = re.search(r"auto-launching \(spawn ([^ )]+) ?([^ )]*) ?([^ )]*)\)", self.boot_log())
            self.roster = (m.group(1), m.group(2) or None, m.group(3) or None) if m else self.wanted(args)
            self.mark_spawn()
        return took

    def log(self):
        # the events and replies land in out.log, and only that is read for marks and replies:
        # the session's stdout mirrors every reply too, and two files read as one put every
        # mark off by the length of the mirror, so a read after a mark reached back into the
        # previous reply (an input's trace then began with the move before it)
        try:
            return open(os.path.join(self.dir, "out.log"), encoding="utf8", errors="replace").read()
        except FileNotFoundError:
            return ""

    def boot_log(self):
        text = ""
        for name in ("boot.log", "out.log"):
            try:
                text += open(os.path.join(self.dir, name), encoding="utf8", errors="replace").read()
            except FileNotFoundError:
                pass
        return text

    def wait(self, needles, timeout):
        """the boot's own lines are on the session's stdout: both files are read for those."""
        t0 = time.time()
        while time.time() - t0 < timeout:
            text = self.boot_log() if self.mark0 == 0 else self.log()[self.mark0:]
            if all(n in text for n in needles):
                return time.time() - t0
            if self.proc and self.proc.poll() is not None:
                return None
            time.sleep(0.2)
        return None

    def await_seq(self, mark, steps, timeout=6.0):
        """the SSF2 session answers a seq once, when it has been scheduled."""
        t0 = time.time()
        while "<< seq " not in self.log()[mark:]:
            left = timeout - (time.time() - t0)
            if left <= 0:
                break
            self.next_line(left)
        return time.time() - t0

    def tell(self, cmd):
        mark = len(self.log())
        with open(os.path.join(self.dir, "control"), "a", encoding="utf8") as f:
            f.write(cmd + "\n")
        return mark

    def _tree_text(self, timeout=10.0):
        """the source's object walk. It has no end marker and the source's eval reply is not
        `E:`-tagged, so completion is read from the feed going quiet: the walk dumps hundreds of
        lines in a burst, then stops. return once the log has not grown for a short beat."""
        mark = self.tell("tree")
        t0 = time.time()
        last_len = len(self.log())
        quiet = 0.0
        while time.time() - t0 < timeout:
            self.next_line(0.15)
            n = len(self.log())
            if n > last_len:
                last_len = n
                quiet = 0.0
            else:
                quiet += 0.15
                # the walk arrived (some lines) and the feed has been quiet a beat -> it is done
                if quiet >= 0.4 and last_len > mark + 40:
                    break
            if self.proc and self.proc.poll() is not None:
                break
        return self.log()[mark:]

    def close(self, final=False):
        if not final:
            return
        if LIVE.get(self.kind) is self:
            del LIVE[self.kind]
        self.feed_stop()
        try:
            self.tell("exit")
            if self.proc:
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
        finally:
            if self.proc:
                self.proc.terminate()
            subprocess.run(["pkill", "-f", "SSF2-patched"], capture_output=True)


# the inputs a fighter is driven with on both engines: a controller input means the same thing
# on both, which a state name does not. a direction pressed with attack is a smash on both and a
# direction held first a tilt (the runtime reads the source's window, Fighters.smashInput)
# a direction pressed on the same frame as special reads as a flick on the source engine (samus's
# side special fires its super missile off it), so the direction is held first
# up special holds up briefly BEFORE adding special so a directional recovery reads the heading:
# goku's teleport goes sideways if up is not already down when special fires (held together it
# defaults to the facing), while a non-directional up special is unaffected by the extra up frames.
C6_INPUTS = ["attack", "special", "right:8 right+special", "up:2 up+special", "down+special", "jump"]
# the rest of what an input can start: the smashes, the tilts and the aerials, driven when the run
# asks for them (`--deep`). they cost about a minute a fighter, so a roster run says which set it
# wants.
C6_INPUTS_MORE = ["right+attack", "up+attack", "down+attack",
                  "right:6 right+attack", "up:6 up+attack", "down:6 down+attack", "jump:3 attack",
                  "jump:3 right+attack", "jump:3 up+attack", "jump:3 down+attack"]
# what this run drives: the six every fighter has, plus the rest when asked
C6_ACTIVE = list(C6_INPUTS)
# what a source speed becomes here when the converter cannot be asked: its size multiplier over
# the frame-rate step (1.3 over 2)
VELOCITY_SCALE = 0.65


# ----------------------------------------------------------------------------------------------
# shared static checks
# ----------------------------------------------------------------------------------------------
def dangling_image_refs(lib):
    have = set()
    for m in glob.glob(os.path.join(lib, "sprites", "**", "*.png.meta"), recursive=True):
        try:
            have.add(json.load(open(m, encoding="utf8"))["guid"])
        except Exception:
            pass
    refs = set()
    for e in glob.glob(os.path.join(lib, "entities", "*.entity")):
        refs.update(re.findall(r'"imageAsset"\s*:\s*"([^"]+)"', open(e, encoding="utf8").read()))
    return sorted(refs - have), len(refs)


def emitted_script_problems(root):
    code, out, _ = run([sys.executable, os.path.join(ROOT, "tools", "check-emitted.py"), root], 300)
    m = re.search(r"(\d+) problem\(s\)", out)
    return (int(m.group(1)) if m else -1), out


def check_assets(plan, r):
    a = plan.get("assets", {})
    r.evidence["assets"] = a
    if a.get("images", 0) > ASSET_BUDGET_IMAGES or a.get("bytes", 0) > ASSET_BUDGET_BYTES:
        r.fail(f"over the publish budget: {a.get('images')} images / {a.get('bytes', 0) / 1e6:.0f}MB "
               f"(limit {ASSET_BUDGET_IMAGES} / {ASSET_BUDGET_BYTES / 1e6:.0f}MB): will not publish")


def check_fra(cid, plan, want_entities, t_convert, r):
    fra = installed_fra(cid)
    if not fra or not os.path.exists(fra):
        r.fail(f"no installed package at {fra}")
        return
    if os.path.getmtime(fra) < t_convert:
        r.fail("the installed package is OLDER than this conversion: the export did not replace it")
        return
    d = read_fra(fra)
    scripts = {s.get("id") for s in d.get("scripts", [])}
    entities = {e.get("id") for e in d.get("entities", [])}
    r.evidence["fra"] = {"scripts": len(scripts), "entities": len(entities),
                         "bytes": os.path.getsize(fra)}
    missing_s = sorted(set(plan.get("scripts", [])) - scripts)
    if missing_s:
        r.fail(f"{len(missing_s)} planned script(s) not in the package: {missing_s[:4]}")
    missing_e = sorted(set(want_entities) - entities)
    if missing_e:
        r.fail(f"{len(missing_e)} planned entit(ies) not in the package: {missing_e[:4]}")
    if not missing_s and not missing_e:
        r.note(f"package carries {len(scripts)} scripts, {len(entities)} entities")


# ----------------------------------------------------------------------------------------------
# stage gates
# ----------------------------------------------------------------------------------------------
def hx_unparsed(pkg, r):
    """every script the package carries, parsed with the Haxe toolchain where it is installed
    (tools/hxcheck): the engine drops a script it cannot parse whole, and what shows first is a
    frame asking for a name that is right there in it"""
    scripts = os.path.join(pkg, "library", "scripts")
    if not shutil.which("haxe") or not os.path.isdir(scripts):
        return
    code, out, _ = run(["haxe", "-cp", os.path.join(ROOT, "tools", "hxcheck"), "--macro", f"HxCheck.run('{scripts}')"], 300)
    bad = [l for l in out.splitlines() if re.match(r"\S+\.hx:\d+: ", l)]
    r.evidence["unparsed"] = bad[:8]
    if bad:
        r.fail(f"{len(bad)} script(s) the engine cannot parse: {bad[0][:120]}")


def s_convert(ctx, r):
    ctx["t_convert"] = time.time()
    shutil.rmtree(ctx["dir"], ignore_errors=True)
    code, out, secs = run([BIN, "ssf2", "stage", ctx["ssf"], "--out", os.path.join(ROOT, "stages")],
                          CONVERT_TIMEOUT_S, env={"RUST_LOG": "info"})
    ctx["convert_log"] = out
    m = re.search(r"stage audit: objects=(\d+) with-own-code=(\d+) borrowed-from-another-package=(\d+) "
                  r"stage-class-ported=(\w+)", out)
    if m:
        r.evidence["source"] = {"objects": int(m.group(1)), "with_own_code": int(m.group(2)),
                                "borrowed": int(m.group(3)), "stage_class_ported": m.group(4) == "true"}
    r.evidence["warnings"] = len(re.findall(r"\bWARN\b", out))
    r.evidence["seconds"] = round(secs)
    if code != 0:
        r.fail(f"converter exit {code}: {out.strip().splitlines()[-1][:140] if out.strip() else 'no output'}")
        return
    plan_path = os.path.join(ctx["dir"], "verify.json")
    if not os.path.exists(plan_path):
        r.fail("no verify.json: the converter wrote no plan")
        return
    plan = json.load(open(plan_path, encoding="utf8"))
    ctx["plan"] = plan
    kinds = {}
    for m_ in plan["made"]:
        kinds[m_["kind"]] = kinds.get(m_["kind"], 0) + 1
    r.evidence["plan"] = kinds
    problems, _ = emitted_script_problems(os.path.join(ctx["dir"], "library", "scripts"))
    if problems:
        r.fail(f"{problems} problem(s) in the emitted scripts (tools/check-emitted.py)")
    hx_unparsed(ctx["dir"], r)
    dangling, nrefs = dangling_image_refs(os.path.join(ctx["dir"], "library"))
    r.evidence["image_refs"] = nrefs
    if dangling:
        r.fail(f"{len(dangling)} image reference(s) resolve to no sprite")
    check_assets(plan, r)
    if r.status == "PASS":
        r.note(f"{len(plan['made'])} objects planned, {len(plan['scripts'])} scripts, "
               f"{plan['assets']['images']} images")


def s_publish(ctx, r):
    if not publish(os.path.join(ctx["dir"], f"{ctx['cid']}.fraytools"), r):
        return
    want = [m["id"] for m in ctx["plan"]["made"] if m["kind"] in ("vfx", "object")]
    check_fra(ctx["cid"], ctx["plan"], want + [ctx["cid"]], ctx["t_convert"], r)


def s_boot(ctx, r):
    eng = Engine(f"stage-{ctx['id']}")
    ctx["engine"] = eng
    took = eng.boot(["--stage", ctx["cid"]], ("engine READY", "LAUNCHED"))
    if took is None:
        r.fail("the engine never launched the match" + eng.crash_reason())
        return
    if eng.wait(("ANIM:STAND",), 90) is None:
        r.fail("the match never reached STAND: the stage wedged the engine at the intro")
        return
    # the engine at its native rate over one frame-paced second: the stage script's own frame
    # counter advancing is the definitive "not wedged", and cpu is sampled across that same
    # window rather than a dedicated wall-clock wait -- the frames elapsing ARE the interval ps
    # needs to attribute cpu. pinned means the frames stopped coming, not that the engine is
    # busy: a stage with six hundred collision structures (pokemon colosseum's painted stands,
    # every step of them solid in the source) runs the engine hot and still delivers every frame
    f0 = eng.eval("__verifyFrames"); c0 = eng.cpu()
    eng.nap(1.0)
    f1 = eng.eval("__verifyFrames"); c1 = eng.cpu()
    cpus = [c for c in (c0, c1) if c is not None]
    r.evidence["cpu"] = cpus
    try:
        delivered = int(f1) - int(f0)
    except (TypeError, ValueError):
        delivered = 0
    pinned = bool(cpus) and all(c > 95 for c in cpus)
    if pinned and delivered < 40:
        r.fail(f"engine pinned at {min(cpus):.0f}%: something is spinning ({delivered} frames in a second)")
    elif pinned:
        r.note(f"engine hot at {min(cpus):.0f}% but delivering frames ({delivered} in a second)")
    # the engine's rate is read at its own speed above; from here the frame-paced gates can run
    # several engine frames a tick and measure the same
    eng.faster(ctx["opts"].get("speed", 1))
    p1 = eng.eval("__verifyFrames")
    eng.nap(1.0)
    p2 = eng.eval("__verifyFrames")
    r.evidence["pulse"] = [p1, p2]
    try:
        if int(p2) <= int(p1):
            r.fail(f"the stage script's frame counter did not advance ({p1} -> {p2}): update is not running")
    except (TypeError, ValueError):
        r.fail(f"the stage script cannot be asked its frame count ({p1!r}): the script is not loaded")
        return
    built = eng.eval("__verifyBuilt")
    r.evidence["built"] = built
    if built != "true":
        r.fail(f"the spawn block never completed (__verifyBuilt={built!r})")
    errs = eng.errors()
    r.evidence["script_errors"] = errs[:8]
    if errs:
        r.fail(f"{len(errs)} script error(s): {errs[0][:110]}")
    if r.status == "PASS":
        seen = [c for c in cpus if c is not None]
        r.note(f"STAND after {took:.0f}s, pulse {p1}->{p2}, cpu {max(seen):.0f}%" if seen
               else f"STAND after {took:.0f}s, pulse {p1}->{p2}, cpu unreadable")


def parse_dump(text):
    out = []
    for rec in (text or "").split(";"):
        if not rec.strip():
            continue
        f = rec.split("|")
        e = {"id": f[0], "k": f[1]}
        if len(f) >= 3 and f[2] in ("null", "gone"):
            e["state"] = f[2]
        else:
            e["state"] = "live"
            e["x"], e["y"] = float(f[2]), float(f[3])
            if len(f) >= 6:
                e["frame"], e["total"] = int(float(f[4])), int(float(f[5]))
            e["held"] = len(f) >= 7 and f[6] == "held"
        out.append(e)
    return out


def s_built(ctx, r):
    eng = ctx["engine"]
    dump = parse_dump(eng.eval("__verifyDump()", timeout=20))
    ctx["dump0"] = dump
    planned = [m for m in ctx["plan"]["made"] if m["kind"] in ("vfx", "object", "structure")]
    r.evidence["planned"], r.evidence["reported"] = len(planned), len(dump)
    if len(dump) != len(planned):
        r.fail(f"the script made {len(dump)} objects and was written to make {len(planned)}")
    by_id = {}
    for e in dump:
        by_id.setdefault(e["id"], []).append(e)
    bad = []
    # a group the stage's own code MOVES (urban champion slides its hole by 900 as the street
    # scrolls) is wherever that code put it, not where the tree placed it
    moved_by_code = set()
    try:
        cid = ctx["cid"]
        with open(os.path.join(ROOT, "stages", cid, "library", "scripts", "stage", f"{cid}Script.hx"), encoding="utf8") as f:
            src = f.read()
        moved_by_code = {c for c in re.findall(r"\b([A-Za-z_]\w*)\.set[XY]\(", src)}
        # ...or, as the runtime path writes it, a clip's place set as a field (`fg.shadow.y = ...`)
        moved_by_code |= {c for c in re.findall(r"\b([A-Za-z_]\w*)\.[xy]\s*=(?!=)", src)}
        # ...or held in a field and placed through it (`fore = fg.gangplankHull`, then `fore.y = ...`,
        # which the runtime writes as a copy of the field's value put back): the clips stored there
        cells = set(re.findall(r"var (__cw\d+) = AS3\.copyOf\(me\.__c_(\w+)\.get\(\)\); \1\.[xy] =", src))
        for _, cell in cells:
            for held in re.findall(r"me\.__c_" + re.escape(cell) + r"\.set\(([^;]*)\);", src):
                moved_by_code |= set(re.findall(r"\.([A-Za-z_]\w*)\)?$", held.strip()))
        # ...or its art was handed to an object's class as a value (saffron city's platform
        # class carries its picture along), which is a clip handle passed inside an ask
        for args in re.findall(r"__ask\([^;]*?\[([^\]]*)\]\)", src):
            moved_by_code |= {re.sub(r"__clip$", "", c) for c in re.findall(r"\b([A-Za-z_]\w*)\b", args)}
    except OSError:
        pass
    # ...or it is an object's own picture (its clip is one the object is placed as), which goes
    # where the object's code takes it
    moved_by_code |= {c for m in planned if m.get("kind") == "object" for c in (m.get("clips") or [])}
    for m in planned:
        cands = by_id.get(m["id"], [])
        if not cands:
            bad.append(f"{m['id']}: never made")
            continue
        e = cands.pop(0)
        if e["state"] != "live":
            bad.append(f"{m['id']}: {e['state']}")
            continue
        if m["kind"] == "vfx":
            # ...under its clip's name or any name over it (a platform's picture is `sprite`
            # inside `movingplatLsprite`, and the code moves the latter)
            code_moves_it = m.get("clip") in moved_by_code or any(n in moved_by_code for n in m.get("names") or []) \
                or bool(m.get("rides"))
            if not code_moves_it and (abs(e["x"] - m["x"]) > 0.6 or abs(e["y"] - m["y"]) > 0.6):
                bad.append(f"{m['id']}: at ({e['x']:.1f},{e['y']:.1f}), planned ({m['x']:.1f},{m['y']:.1f})")
            if m.get("frames") and e.get("total") != m["frames"]:
                bad.append(f"{m['id']}: clock {e.get('total')} frames, planned {m['frames']}")
    r.evidence["mismatches"] = bad[:12]
    if bad:
        r.fail(f"{len(bad)} object(s) not as planned: {bad[0]}")
    for w in [m for m in ctx["plan"]["made"] if m["kind"] == "weather"]:
        n = eng.eval(f"{w['var']}.length")
        r.evidence.setdefault("weather", {})[w["id"]] = n
        try:
            if int(n) < 1:
                # the stage's own code fills its weather clip, and may leave it empty this match (a
                # rain on a roll of the dice): nothing in the clip is the code's choice, clips in it
                # with nothing drawn for them is the port's fault
                held = eng.eval("Weather.layer == null ? 0 : Weather.layer.__kids.length")
                if str(held) == "0":
                    r.note(f"{w['id']}: the stage's code put no weather in its clip this match")
                else:
                    r.fail(f"{w['id']}: no particles drawn for {held} clip(s) in the weather clip (planned {w['count']})")
        except (TypeError, ValueError):
            r.fail(f"{w['id']}: field unreadable ({n!r})")
    if r.status == "PASS":
        r.note(f"{len(planned)} objects made, every one where and how it was planned")


def s_motion(ctx, r):
    eng = ctx["engine"]
    # the frame count and every clock in ONE read, so each sample is one instant of the stage
    def sample():
        got = eng.eval('__verifyFrames + "|" + __verifyDump()', timeout=20)
        if got is None:
            raise RuntimeError("the stage did not answer its object dump" + (eng.crash_reason() or " (no reply in 20 s)"))
        p, dump = got.split("|", 1)
        d = parse_dump(dump)
        return int(p), {e["id"] + "#" + str(i): e for i, e in enumerate(d)}
    samples = [sample()]
    eng.nap(0.8)
    samples.append(sample())
    eng.nap(0.9)
    samples.append(sample())
    (p0, s0), (p1, s1), (p2, s2) = samples
    r.evidence["frames_run"] = [p1 - p0, p2 - p1]
    if p1 <= p0 or p2 <= p1:
        r.fail("the stage stopped counting frames between samples: the engine froze")
        return
    off, loops, moved, driven = [], 0, 0, 0
    # a picture the stage's own code sends to a frame (crystal smash shows a platform's holder
    # by the platform's height), or that rides an object whose class picks its frame, keeps the
    # frame the code picks, not the engine's count: the plan says which, and the stage's own
    # report marks one its code holds by a name worked out as it runs
    driven_ids = {m["id"] for m in ctx["plan"]["made"] if m.get("id") and m.get("driven")}
    for k, e in s0.items():
        if e.get("state") != "live":
            continue
        a, b = s1.get(k), s2.get(k)
        if e["k"] == "v" and (e["id"] in driven_ids or e.get("held") or (b and b.get("held"))):
            driven += 1
            continue
        # only a looping picture is held to one frame per frame. a game object runs its own
        # state machine: it holds a last frame, switches animation, waits on a timer, and its
        # clock is its business (bowsers castle's thwomp idles 106 frames and then drops)
        if e["k"] == "v" and e.get("total", 1) > 1 and a and b:
            loops += 1
            # a looping clock advances one frame per frame the stage runs; the sample can land
            # a frame either side of the read, so one frame of slack, and no wall clock anywhere
            def in_step(f0, fp, dp, tot):
                return min((fp - (f0 + dp)) % tot, (f0 + dp - fp) % tot) <= 1
            ok1 = in_step(e["frame"], a["frame"], p1 - p0, e["total"])
            ok2 = in_step(e["frame"], b["frame"], p2 - p0, e["total"])
            if not (ok1 or ok2):
                off.append(f"{e['id']}: frames {e['frame']}->{a['frame']}->{b['frame']} of {e['total']} "
                           f"over {p1 - p0},{p2 - p0} frames")
        if e["k"] == "o" and b and (abs(e["x"] - b["x"]) > 0.5 or abs(e["y"] - b["y"]) > 0.5):
            moved += 1
    r.evidence["loops"], r.evidence["objects_moved"], r.evidence["out_of_step"], r.evidence["driven"] = loops, moved, off[:12], driven
    if off:
        r.fail(f"{len(off)} loop(s) not in step with the stage's frames: {off[0]}")
    else:
        r.note(f"{loops} loops in step over {p2 - p0} frames, {moved} object(s) moving on their own"
               + (f", {driven} picture(s) on the frame the stage's code picks" if driven else ""))


def s_ssf2(ctx, r):
    if not ctx["opts"].get("ssf2"):
        r.skip("not requested (--ssf2)")
        return
    # the comparison runs in-process on the engines already up: the Fraymakers one is on this
    # stage from the boot gate, and the source's side starts its match in the SSF2 engine the
    # run holds (booting it only when none is up)
    sys.path.insert(0, os.path.join(ROOT, "tools", "tests"))
    import io, contextlib
    import stage_cross_engine as sce
    os.environ["PEPTIDE_SCE_DUMP"] = "1"
    t0 = time.time()
    s = ssf2_up(ctx, r)
    if s is None:
        return
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            bad_list = sce.run(ctx["id"], ctx.get("engine"), s)
        code = 1 if bad_list else 0
    except SystemExit as e:
        code = 1
        buf.write(f"\nstage_cross_engine stopped: {e}\n")
    out = buf.getvalue()
    r.evidence["seconds"] = round(time.time() - t0)
    r.evidence["tail"] = out.strip().splitlines()[-12:]
    # the whole report, beside the gate's own: the verdicts are at its head
    os.makedirs(os.path.join(ROOT, "build", "verify", "stage"), exist_ok=True)
    with open(os.path.join(ROOT, "build", "verify", "stage", f"{ctx['id']}.s6.txt"), "w", encoding="utf8") as f:
        f.write(out)
    bad = [l for l in out.splitlines() if re.search(r"MISSING|COUNT \d+ in FM|LENGTH MISMATCH|LAYER MISMATCH|<-- OFF|SEGMENTED|RATE OFF", l)]
    r.evidence["verdicts"] = bad[:12]
    if code != 0:
        r.fail(f"stage_cross_engine.py reported differences: {bad[0].strip()[:120] if bad else 'see the report'}")
    else:
        r.note("both engines animate the same objects the same way")


def ssf2_up(ctx, r):
    """the source engine on this stage, booted once for the run and shared by every gate that
    reads it (the stand and object comparisons)"""
    s = ctx.get("ssf2")
    if s is not None:
        return s
    sys.path.insert(0, os.path.join(ROOT, "tools", "tests"))
    import stage_cross_engine as sce
    boot_thread = ctx["opts"].get("ssf2_boot")
    if boot_thread is not None:
        boot_thread.join()
    s = Ssf2Engine(f"ssf2-stage-{ctx['id']}")
    if s.boot(["--char", f"{sce.CHARACTER},{sce.CHARACTER}", "--stage", ctx["id"]]) is None:
        r.fail("the SSF2 session never came up")
        return None
    # run the source at the engine's top rate: every gate reads it on its frame count, so the
    # measurement is identical and only arrives sooner. this matters most for the stand gate's
    # clock sync, which can tick the source hundreds of frames to catch up to the faster
    # fraymakers side -- at the native rate that catch-up is the slow part of a complex stage's
    # gate (steel diver, bowser's castle sat minutes there); at 8x it is seconds
    s.faster(8)
    ctx["ssf2"] = s
    return s


def fm_elapsed(eng, tries=4):
    """the match's frame count on Fraymakers. a read can go unanswered while the engine runs a
    heavy frame (a stage's minigame building its pieces); it is asked again before giving up"""
    for _ in range(tries):
        v = eng.eval("match.getElapsedFrames()")
        try:
            return int(float(v))
        except (TypeError, ValueError):
            eng.tick(2)
    raise RuntimeError("the engine did not answer its frame count " + str(tries) + " times" + (eng.crash_reason() or ""))


def ssf2_read(eng, expr):
    """one number off the source engine's property reader, taken as the first reply after the
    session's echo of this very ask (a reply to an earlier one can land after it was sent)"""
    mark = eng.tell(expr)
    for _ in range(100):
        text = eng.log()[mark:]
        # the echo of THIS ask, whole: a write of the same name (`e p0.Y = 5`) echoes with the
        # read as its prefix, and its `ok` is not a number
        i = text.rfind(">> " + expr + "\n")
        if i >= 0:
            m = re.search(r"<< (.+)", text[i + len(expr) + 3:])
            if m:
                return float(m.group(1).strip())
        eng.next_line(0.1)
    raise ValueError(f"no answer to {expr}")


def s_stand(ctx, r):
    """where a fighter comes to rest, engine against engine: each fighter is lifted a little over
    its entrance in both engines and let fall, and the source's resting height goes through the
    port's own coordinate model. a floor the port has and the source has not (or the other way
    round) shows here as a fighter standing in the air or through the ground."""
    if not ctx["opts"].get("ssf2"):
        r.skip("not requested (--ssf2)")
        return
    eng = ctx.get("engine")
    if eng is None:
        r.fail("the Fraymakers engine has to be up (the boot gate)")
        return
    s = ssf2_up(ctx, r)
    if s is None:
        return
    plan = ctx["plan"]
    # the space a fighter's position is read in: the plan states it (the terrain's registration,
    # which the source lays its world out from), apart from the one its objects do their sums in
    scale, origin, ents = plan.get("scale"), plan.get("fighter_origin") or plan.get("origin"), plan.get("entrances") or []
    if not scale or not origin or not ents:
        r.fail("the plan carries no coordinate model (scale, fighter_origin, entrances)")
        return
    ents = sorted(ents[:2], key=lambda e: float(e["x"]))
    r.evidence["origin"] = {"x": round(origin[0], 2), "y": round(origin[1], 2), "scale": scale}
    r.evidence["entrances"] = [[round(float(e["x"]), 1), round(float(e["y"]), 1)] for e in ents]
    # port px over the entrance the fall starts from: enough to fall onto whatever the entrance
    # stands on, and no more, since an entrance the source puts under a platform (sky
    # sanctuary's sit 68 px below the platforms' ends) is meant to land on the floor beneath;
    # the second drop, from well above, is for an entrance a plate holds inside it
    LIFT = 40.0
    TOL = 16.0    # a ledge lip or a sagging platform is a few px; a missing floor is a hundred
    # every fighter goes at once, each over its own entrance, so both engines hold the same
    # configuration while they settle: a platform that sags under weight sags under the same
    # weight, and nobody is read while the other fighter still stands somewhere else
    rows, bad, falling, frozen = [], [], set(), set()
    # twice, a stretch of frames apart: an object that moves across the fall (a passing
    # platform, a bird) lands a fighter somewhere else for a moment on one engine, and a
    # difference is one that holds both times
    def sync_clocks():
        """bring both engines to the same moment of the match (in the source's frames) before
        anything is dropped: fourside's travelling platform sat at opposite ends of its run in
        the two engines, and the drop landed on it in one and past it in the other"""
        try:
            fm = fm_elapsed(eng)
        except (TypeError, ValueError):
            return None
        src = s.frames_since_spawn()
        diff = int(round(fm / 2 - src))
        if diff > 2:
            s.tick(diff)
        elif diff < -2:
            eng.tick(-diff * 2)
        return {"fraymakers": fm, "source": src, "ticked": diff}
    def attempt():
        nonlocal LIFT
        rows, bad = [], []
        placed_mark = len(s.log())
        placed_y = {}
        r.evidence.setdefault("sync", []).append(sync_clocks())
        for k, e in enumerate(ents):
            fx, fy = float(e["x"]), float(e["y"]) - LIFT
            # and falling: a fighter set down at a height in its stand state is not attached
            # to anything and sinks through a drop-through platform (lunar core's entrance
            # platform caught the source and not the port on that account); one that falls
            # lands the way the source's does
            eng.eval(f"match.getCharacters()[{k}].setX({fx:.2f})")
            eng.eval(f"match.getCharacters()[{k}].setY({fy:.2f})")
            # two statements: the `e` line takes them as such (eval would wrap them as one)
            # and still across: a fighter keeps the speed it had where it was (walking, knocked by
            # a hazard), and drifts onto another floor as it falls (konoha's steps took one 90 px
            # over from the source's)
            eng.reply(eng.tell(f"e match.getCharacters()[{k}].setYVelocity(1); match.getCharacters()[{k}].setXVelocity(0)"), "E:", 5)
            eng.reply(eng.tell(f"e match.getCharacters()[{k}].toState(CState.FALL); match.getCharacters()[{k}].setXVelocity(0)"), "E:", 5)
            sx, sy = fx / scale + origin[0], fy / scale + origin[1]
            if k == 0:
                placed_mark = len(s.log())
            placed_y[k] = sy
            s.tell(f"e p{k}.X = {sx:.2f}")
            s.tell(f"e p{k}.Y = {sy:.2f}")
        # the port's height is read at the LANDING, the first moment every dropped fighter is on
        # a floor: read later, a fighter that slid down a slope (each fighter has its own
        # traction, and the port's control fighter is not the source's) reports the slope's foot
        # a fighter that dropped into a death zone (lava, a pit, deep water) is dead and gone, so
        # getY() comes back with nothing: read it defensively and let None mean "no fighter here"
        # rather than crashing the whole gate on float(None)
        def read_y(k):
            try:
                return float(eng.eval(f"match.getCharacters()[{k}].getY()"))
            except (TypeError, ValueError):
                return None
        # the source's landing too, in the same loop: the first height it holds still at after
        # it fell. a stage that moves a fighter later on its own clock (rainbow route's mirrors
        # take one every twelve seconds and drop it at the gold mirror) does it at another moment
        # on each engine, and the landings are the rests that compare
        # the engines run free between the harness's reads, so the source's landing is looked for
        # only while the port's fighters come down and a few reads after: every read is time the
        # source runs on (a scrolling stage carries its fighter far off meanwhile)
        landed, src_landed, src_prev, after = {}, {}, {}, 0
        for _ in range(40):
            eng.tick(4)
            s.tick(2)
            for k in range(len(ents)):
                if k not in landed and eng.eval(f"match.getCharacters()[{k}].isOnFloor()") == "true":
                    y = read_y(k)
                    if y is not None:
                        landed[k] = y
                if k not in src_landed:
                    try:
                        y = ssf2_read(s, f"e p{k}.Y")
                    except ValueError:
                        y = None
                    prev = src_prev.get(k)
                    if y is not None and prev is not None and abs(y - prev) < 0.5 and abs(y - placed_y.get(k, y)) > 0.5:
                        src_landed[k] = y
                    src_prev[k] = y
            if len(landed) == len(ents):
                after += 1
                if len(src_landed) == len(ents) or after > 3:
                    break
        # ...and once more at ONE MOMENT OF THE MATCH on both engines: the two run free while
        # the harness talks to either, so a rest on something that moves (dracula's castle's
        # platforms carry the fighters through a cycle) is only the same rest when both are
        # read at the same frame. each engine is walked to that frame on its own clock
        try:
            fm_now, src_now = fm_elapsed(eng), s.frames_since_spawn()
        except (TypeError, ValueError):
            fm_now, src_now = 0, 0
        at = max(fm_now // 2, src_now) + 90
        def walk_to(engine, frame, read, per):
            for _ in range(40):
                now = read()
                if now is None or now >= frame:
                    return now
                engine.tick(min((frame - now) * per, 120 * per))
            return read()
        fm_at = walk_to(eng, at, lambda: fm_elapsed(eng) // 2, 2)
        src_at = walk_to(s, at, s.frames_since_spawn, 1)
        port_at = {k: read_y(k) for k in range(len(ents))}
        r.evidence.setdefault("read_at", []).append({"frame": at, "fraymakers": fm_at, "source": src_at})
        # ...and read a few times over a stretch: a source that holds nothing under its own
        # entrance (a game-mode stage whose ground waits for the mode: crystal smash's starter
        # platform is fall-through until its mode starts) lets the fighter fall and respawn
        # forever, and a height read off a falling fighter is not a rest to compare with. the
        # stretch outlasts the revival platform's hold, which reads as a rest on its own. a
        # fighter that moves but never respawns is riding something (dracula's castle's
        # platforms carry theirs), which is a rest the port has to match
        reads = {k: [ssf2_read(s, f"e p{k}.Y")] for k in range(len(ents))}
        for _ in range(4):
            s.tick(20)
            for k in range(len(ents)):
                reads[k].append(ssf2_read(s, f"e p{k}.Y"))
        revived = "ANIM:REVIVAL" in s.log()[placed_mark:]
        for k, e in enumerate(ents):
            port_y = landed.get(k)
            if port_y is None:
                port_y = read_y(k)
            src_y = reads[k][0]
            src_as_port = (src_y - origin[1]) * scale
            # the port's fighter left no height to read: it dropped into a death zone at this
            # entrance and died. that is a death-zone entrance (like lava or a pit), not a rest to
            # compare, so treat it as a falling one -- skipped by the same path, and not retried
            # from a higher lift (it would only die again).
            if port_y is None:
                falling.add(k)
                rows.append({"entrance": k, "lift": LIFT, "port": None,
                             "source": round(src_y, 2), "source_as_port": round(src_as_port, 1),
                             "port_died": True})
                bad.append(f"entrance {k}: the port's fighter dropped into a death zone and died (no height to read)")
                continue
            pa = port_at.get(k)
            # the source at the shared moment is its first read; the later ones say whether it
            # holds still there
            sl = src_landed.get(k)
            sl_as_port = (sl - origin[1]) * scale if sl is not None else None
            row = {"entrance": k, "lift": LIFT, "port": round(port_y, 1),
                   "port_at": round(pa, 1) if pa is not None else None,
                   "source": round(src_y, 2), "source_as_port": round(src_as_port, 1),
                   "source_landed_as_port": round(sl_as_port, 1) if sl_as_port is not None else None}
            # the source landed at one height and is at another a moment later: the stage moved
            # it on its own clock (rainbow route's mirrors take a fighter and drop it at the gold
            # mirror), at another moment than the port's, and the entrance has no rest to compare
            if sl_as_port is not None and abs(sl_as_port - src_as_port) > TOL \
                    and abs(port_y - sl_as_port) > TOL and (pa is None or abs(pa - src_as_port) > TOL):
                row["stage_moved"] = True
            if max(reads[k]) - min(reads[k]) > 8 and revived:
                row["source_falling"] = True
                falling.add(k)
            elif max(reads[k]) - min(reads[k]) > 8:
                row["source_moving"] = True
            # a fighter still exactly where it was put, in the air, never fell: the source
            # engine is not running its match (rest area's scripts throw every frame outside
            # the mode they read), and there is no rest to compare with
            elif abs(src_y - placed_y.get(k, src_y)) < 0.5:
                row["source_frozen"] = True
                frozen.add(k)
            rows.append(row)
            # the landing read answers a still floor, the shared-moment read a moving one (when it
            # could be read: a fighter gone by the shared moment leaves only the landing to judge)
            if abs(port_y - src_as_port) > TOL and (pa is None or abs(pa - src_as_port) > TOL) \
                    and (sl_as_port is None or abs(port_y - sl_as_port) > TOL):
                at_txt = f" (at the shared moment {pa:.0f})" if pa is not None else ""
                bad.append(f"entrance {k}: the port rests at y={port_y:.0f}{at_txt}, the source at {src_as_port:.0f} (its {src_y:.1f})")
        # and the landings are let pass before anything else reads the stage: a landing's dust
        # is a clip of its own for a moment, and the object comparison after this would count it
        eng.tick(120)
        s.tick(60)
        return rows, bad
    try:
        rows, bad = attempt()
        # a fighter that died and respawned at its drop (a death-zone entrance: lava, a pit)
        # dies again from a higher lift, so the retry only spends time reaching the same skip
        # verdict the end already gives it. retry only when a failing entrance is a real
        # comparison, not one that is already falling
        # (and an entrance the stage moved its fighter at is no comparison to retry)
        moved_ks = {w["entrance"] for w in rows if w.get("stage_moved")}
        real = [b for b in bad if not any(b.startswith(f"entrance {k}:") for k in moved_ks)]
        bad_ks = {int(m.group(1)) for b in real if (m := re.match(r"entrance (\d+):", b))}
        if real and not (bad_ks and bad_ks <= falling):
            eng.tick(200)
            s.tick(100)
            # ...and from well above: an entrance under a platform (lunar core's sits 128 px
            # below one) starts the first drop inside the plate, where the source pushes its
            # fighter up and this engine lets it fall. a difference has to hold from clear above.
            LIFT = 240.0
            again, bad2 = attempt()
            r.evidence["first_attempt"] = rows
            r.evidence["second_attempt"] = again
            # each entrance is judged from the lift that cleared it: the higher drop lands a
            # source fighter somewhere else again (lunar core's second entrance drifts off a
            # ledge from twice the height), so one lift answering for an entrance is enough
            def agreed(w):
                if w.get("port") is None:
                    return False   # died at this entrance: nothing to agree on
                cands = [abs(w["port"] - w["source_as_port"])]
                pa = w.get("port_at")
                if pa is not None:
                    cands.append(abs(pa - w["source_as_port"]))
                if w.get("source_landed_as_port") is not None:
                    cands.append(abs(w["port"] - w["source_landed_as_port"]))
                return min(cands) <= TOL
            merged, bad3 = [], []
            for w1, w2 in zip(rows, again):
                if agreed(w1) or w1.get("stage_moved"):
                    merged.append(w1)
                elif agreed(w2):
                    merged.append(w2)
                else:
                    merged.append(w2)
                    bad3.append(next(b for b in bad2 if b.startswith(f"entrance {w2['entrance']}:")))
            rows, bad = merged, bad3
    except Exception as ex:
        bad.append(f"the fighters could not be read ({ex!r})")
    r.evidence["rest"] = rows
    # a source fighter that never came to rest in the window (a stage whose own phases carry it:
    # castle siege's transitions, a scrolling or rising ground) has no rest to compare
    moving = {r["entrance"] for r in rows if r.get("source_moving")}
    stage_moved = {r["entrance"] for r in rows if r.get("stage_moved")}
    falling = set(falling) | moving
    # an entrance whose fighter the stage moved mid-fall is no comparison; the rest still are
    bad = [b for b in bad if not any(b.startswith(f"entrance {k}:") for k in stage_moved)]
    if stage_moved:
        r.evidence["stage_moved"] = sorted(stage_moved)
    unheld = [k for k in sorted(falling) if any(b.startswith(f"entrance {k}:") for b in bad)]
    still = [k for k in sorted(frozen) if any(b.startswith(f"entrance {k}:") for b in bad)]
    if bad and len(unheld) == len(bad):
        why = "it was still moving when read (the stage's own phases carry it)" if unheld[0] in moving else "it falls and respawns there (a stage whose ground waits for its game mode)"
        r.skip(f"the source holds no fighter at rest at entrance {unheld[0]}: {why}; nothing to compare")
    elif bad and len(still) + len(unheld) == len(bad):
        r.skip(f"the source's fighter at entrance {still[0]} stayed exactly where it was put, in the air: its engine is not running the match (a stage whose scripts throw outside their mode); nothing to compare")
    elif bad:
        r.fail(bad[0])
    elif rows and all(w.get("stage_moved") for w in rows):
        r.skip("the stage moved every fighter on its own clock before it came to rest (a scrolling ground, a hazard that carries it off); nothing to compare")
    else:
        r.note("; ".join(f"entrance {w['entrance']} moved by the stage as it fell (not compared)" if w.get("stage_moved")
                         else f"entrance {w['entrance']} rests at {w['port']:.0f} in both" for w in rows))


def s_quiet(ctx, r):
    """the match ran every gate above without a script error. S3 counts the boot's; this one
    counts them all, since a stage's code can throw once a minigame or a phase starts (a gate
    that only reads where things stand cannot tell a frame that threw from one that ran)"""
    eng = ctx.get("engine")
    if eng is None:
        r.skip("no match to read")
        return
    errs = eng.errors()
    distinct = sorted({re.sub(r"^<< (SCRIPTLOG: \[ Error \] )?", "", e) for e in errs})
    r.evidence["errors"] = len(errs)
    r.evidence["distinct"] = distinct[:8]
    if errs:
        r.fail(f"{len(errs)} script error(s) while the gates ran ({len(distinct)} distinct): {distinct[0][:110]}")
    else:
        r.note("no script error in the whole match")


STAGE_GATES = [("S1 convert", s_convert), ("S2 publish", s_publish), ("S3 boot", s_boot),
               ("S4 built", s_built), ("S5 motion", s_motion), ("S6 stand", s_stand), ("S7 ssf2", s_ssf2),
               ("S8 quiet", s_quiet)]


def verify_stage(sid, opts):
    cid = f"{sid}ssf2"
    ctx = {"id": sid, "cid": cid, "ssf": source_ssf(sid, "stage"),
           "dir": os.path.join(ROOT, "stages", cid), "opts": opts}
    print(f"== stage {sid}", flush=True)
    if not os.path.exists(ctx["ssf"]):
        print(f"  no source at {ctx['ssf']}")
        return None
    results = run_gates_with_ahead(("stage", sid), STAGE_GATES, ctx)
    if "engine" in ctx:
        if ctx.get("engine"):
            ctx["engine"].close()
    if not opts.get("keep"):
        # the package goes when the stage is done with: the whole corpus kept at once fills the disk
        shutil.rmtree(ctx["dir"], ignore_errors=True)
    return results


# ----------------------------------------------------------------------------------------------
# character gates
# ----------------------------------------------------------------------------------------------
def c_convert(ctx, r):
    import timing_parity
    ctx["t_convert"] = time.time()
    shutil.rmtree(ctx["dir"], ignore_errors=True)
    code, out, secs = run([BIN, "convert", ctx["ssf"], "-n", ctx["id"], "-o", os.path.join(ROOT, "characters"), "-y"],
                          CONVERT_TIMEOUT_S,
                          env={"PEPTIDE_DUMP_ANIM_SPLITS": "1", "PEPTIDE_DUMP_ANIM_LABELS": "1",
                               "PEPTIDE_DUMP_BOX_FRAMES": "1", "DUMP_PARITY": "1"})
    r.evidence["seconds"] = round(secs)
    if code != 0:
        r.fail(f"converter exit {code}: {out.strip().splitlines()[-1][:140] if out.strip() else 'no output'}")
        return
    plan_path = os.path.join(ctx["dir"], "verify.json")
    if not os.path.exists(plan_path):
        r.fail("no verify.json: the converter wrote no plan")
        return
    plan = json.load(open(plan_path, encoding="utf8"))
    ctx["plan"] = plan
    stats = json.load(open(os.path.join(ctx["dir"], "conversion_stats.json"), encoding="utf8"))
    r.evidence["source"] = {"animations": stats.get("animations"), "attacks": stats.get("attacks_extracted"),
                            "frame_scripts": stats.get("frame_scripts")}
    # 30 -> 60fps: every emitted animation is exactly twice the source range it was cut from
    sp = timing_parity.parse_splits(out)
    fm = timing_parity.entity_lengths(ctx["id"], os.path.join(ROOT, "characters"))
    ctx["fm_lengths"] = fm
    off = []
    for name, got in sorted(fm.items()):
        info = sp.get(name)
        if not info or info["frames"] == 0 or info["loop"]:
            continue
        want = info["frames"] * 2
        if abs(got - want) > 1:
            off.append(f"{name}: {got} frames, source {info['frames']} -> expected {want}")
    r.evidence["timing"] = {"checked": len([n for n in fm if n in sp]), "off": off[:8]}
    if off:
        r.fail(f"{len(off)} animation(s) off the 2x doubling: {off[0]}")
    # hitbox stats, value for value against the source dump
    pc, pout, _ = run([sys.executable, os.path.join(ROOT, "tools", "tests", "parity_check.py"), ctx["id"]], 300)
    r.evidence["hitbox_parity"] = "PARITY OK" if "PARITY OK" in pout else pout.strip().splitlines()[-3:]
    if "PARITY OK" not in pout and "SKIP" not in pout:
        r.fail("hitbox stats diverge from the source (tools/tests/parity_check.py)")
    problems, _ = emitted_script_problems(os.path.join(ctx["dir"], "library", "scripts"))
    if problems:
        r.fail(f"{problems} problem(s) in the emitted scripts (tools/check-emitted.py)")
    hx_unparsed(ctx["dir"], r)
    dangling, nrefs = dangling_image_refs(os.path.join(ctx["dir"], "library"))
    r.evidence["image_refs"] = nrefs
    if dangling:
        r.fail(f"{len(dangling)} of {nrefs} image reference(s) resolve to no sprite")
    check_assets(plan, r)
    if r.status == "PASS":
        r.note(f"{len(fm)} animations, timing 2.00x, hitboxes at parity, {nrefs} image refs resolve")


def publish(project, r):
    """the package, built in-process by `peptide pack` (FrayTools is not involved); `--fraytools`
    drives FrayTools' own publish instead, for checking the two agree."""
    if os.environ.get("PEPTIDE_PUBLISH") == "fraytools":
        code, out, secs = run([BIN, "export", "--project", project, "--timeout", str(EXPORT_TIMEOUT_MS)],
                              EXPORT_TIMEOUT_MS / 1000 + 120)
    else:
        code, out, secs = run([BIN, "pack", "--project", project], 300)
    r.evidence["seconds"] = round(secs, 1)
    if code != 0:
        r.fail(f"publish failed: {out.strip().splitlines()[-1][:140] if out.strip() else 'no output'}")
        return False
    return True


def c_publish(ctx, r):
    if not publish(os.path.join(ctx["dir"], f"{ctx['id']}.fraytools"), r):
        return
    check_fra(ctx["id"], ctx["plan"], [ctx["id"]], ctx["t_convert"], r)


PULSE = "match.getCharacters()[0].exports.__verifyPulse()"


def c_boot(ctx, r):
    eng = Engine(f"char-{ctx['id']}")
    ctx["engine"] = eng
    took = eng.boot(["--char", ctx["id"], "--stage", CHAR_STAGE_FM], ("engine READY", "LAUNCHED"))
    if took is None:
        r.fail("the engine never launched the match" + eng.crash_reason())
        return
    if eng.wait(("ANIM:STAND",), 90) is None:
        r.fail("the fighter never reached STAND")
        return
    # where the fighter stands at the start of its match, off its own record once still: the
    # floor the later gates put it back on (a walk can leave it on another level of the stage)
    if standing(eng, 2):
        y = floor_spot(eng)
        if y is not None:
            ctx["home_x"], ctx["home_y"] = 0.0, y
    p1 = eng.eval(PULSE)
    eng.nap(1.0)
    p2 = eng.eval(PULSE)
    r.evidence["pulse"] = [p1, p2]
    try:
        if int(p2) <= int(p1):
            r.fail(f"the fighter's script frame counter did not advance ({p1} -> {p2})")
    except (TypeError, ValueError):
        r.fail(f"the fighter's script cannot be asked its frame count ({p1!r}): the script is not running")
        return
    errs = eng.errors()
    r.evidence["script_errors"] = errs[:8]
    if errs:
        r.fail(f"{len(errs)} script error(s): {errs[0][:110]}")
    # the script checks its own tables against the entity it runs on, in its own scope: every
    # animation name a table routes to exists (see haxe_gen::with_self_check)
    mk = eng.tell("e@p0 __selfCheck()")
    check = (eng.reply(mk, "E:", 10) or "").strip()
    r.evidence["self_check"] = check
    if check and not check.startswith("ok"):
        r.fail(f"the script's own tables name animations the entity lacks: {check[:160]}")
    if r.status == "PASS":
        r.note(f"STAND after {took:.0f}s, pulse {p1}->{p2}, self-check {check or 'not answered'}")


def declared_stats(ctx):
    files = glob.glob(os.path.join(ctx["dir"], "library", "scripts", "*", "CharacterStats.hx"))
    if not files:
        return {}
    text = open(files[0], encoding="utf8").read()
    return {k: float(v) for k, v in re.findall(r"^\s*(\w+):\s*(-?[\d.]+)", text, re.M)}


def trace_runs(text):
    """the engine's own per-frame record, run-length encoded: [(ANIM, frames), ...]."""
    runs = []
    for line in text.splitlines():
        # `  <label> xN` from either engine: one prefixes the row with `<< `, the other appends
        # where the run ended
        m = re.match(r"\s*(?:<<\s+)?(\S+) x(\d+)(?:\s+end=.*)?\s*$", line)
        if m and not line.strip().startswith(">>"):
            runs.append((m.group(1), int(m.group(2))))
    return runs


def held_moves(ctx):
    """the animations whose length an input decides, not the clock: a charge, an aim-and-fire, a
    steerable or looping move that runs for exactly as long as the button is held (zelda's side
    special charges until it goes; chibirobo aims the blaster while special is down). launched with
    no input, such a move fires at once or plays a stub, so its single-dispatch length is not the
    same fact on the two engines. three content-agnostic tells, any one is enough:
      - a frame script jumps back to an earlier label of its own (an in-timeline loop: a reflector,
        a steerable recovery), or
      - a charge: the move stalls at a `charging` label (the SSF2 charge protocol the source emits)
        while a frame script samples the held input (`getHeldControls()`), or
      - a frame script installs a timer (`addTimer(..., cb)`) whose callback samples the held input
        and ends the move on it (`__endAttack()` or `playFrameLabel()` to its wind-down): the move
        polls the button each tick and stops when it goes, so its length is the hold (simon's whip
        spins while attack is held, then plays its endlag). a combo instead ADVANCES to the next
        attack on a follow-up press, and does so with `_playAnimation`/`_jumpLabel`, not by ending
        itself, so a jab that chains stays length-compared.
    C5/C6 check such a move plays the right thing but do not length-compare it."""
    # a character-script function that reads held input each tick and ENDS the move on it. the flow
    # calls that END a move (endAttack, or a jump to its own wind-down) are held; the ones that go to
    # ANOTHER attack (_playAnimation / _jumpLabel) are a combo advance, and are not counted.
    flow = ("__endAttack(", "playFrameLabel(")
    poll_cbs = set()
    for sp in glob.glob(os.path.join(ctx["dir"], "library", "scripts", "*", "Script.hx")):
        src = open(sp, encoding="utf8").read()
        for m in re.finditer(r"function\s+(\w+)\s*\([^)]*\)\s*\{", src):
            i = m.end() - 1
            depth = 0
            j = i
            while j < len(src):
                if src[j] == "{":
                    depth += 1
                elif src[j] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            body = src[i:j]
            if "getHeldControls(" in body and any(f in body for f in flow):
                poll_cbs.add(m.group(1))
    out = set()
    for p in glob.glob(os.path.join(ctx["dir"], "library", "entities", f"{ctx['id']}.entity")):
        d = json.load(open(p, encoding="utf8"))
        kf = {k["$id"]: k for k in d["keyframes"]}
        layers = {l["$id"]: l for l in d["layers"]}
        for a in d["animations"]:
            labels, scripts, charging, samples_held = {}, [], False, False
            for lid in a["layers"]:
                layer = layers.get(lid) or {}
                pos = 0
                for kid in layer.get("keyframes", []):
                    k = kf.get(kid, {})
                    if layer.get("type") == "LABEL" and k.get("name"):
                        labels[k["name"]] = pos
                        if k["name"] == "charging":
                            charging = True
                    elif layer.get("type") == "FRAME_SCRIPT" and k.get("code"):
                        scripts.append((pos, k["code"]))
                        if "getHeldControls(" in k["code"]:
                            samples_held = True
                    pos += k.get("length", 1)
            # a `charging` label IS the SSF2 charge protocol, and that protocol is driven from the
            # character script's own update (`if xframe == "charging" ... getHeldControls().SPECIAL`),
            # not the animation's frame scripts -- so the poll need not sit in THIS animation. the
            # label alone means the move charges while special is held, and its length is the hold
            # (black mage's warp charges the teleport, so special_up_air runs as long as you aim).
            if charging:
                out.add(a["name"])
            blob = "\n".join(c for _, c in scripts)
            wired = {m.group(1) for m in re.finditer(r"addTimer\([^)]*?,\s*(\w+)", blob)}
            if wired & poll_cbs:
                out.add(a["name"])
            for pos, code in scripts:
                for m in re.findall(r'playFrameLabel\(\s*"([^"]+)"\s*\)', code):
                    if m in labels and labels[m] <= pos:
                        out.add(a["name"])
    return out


def transform_moves(ctx):
    """animations that TRANSFORM the fighter into another (zelda into sheik). Fraymakers has no
    character-swap, so the converter strikes the call (`// [no counterpart: replaceCharacter]`)
    and the move cannot be reproduced. Read the strike back so a cross-engine check skips the move
    rather than reading the source's transform as a length/behaviour difference."""
    out = set()
    for p in glob.glob(os.path.join(ctx["dir"], "library", "entities", f"{ctx['id']}.entity")):
        d = json.load(open(p, encoding="utf8"))
        kf = {k["$id"]: k for k in d["keyframes"]}
        layers = {l["$id"]: l for l in d["layers"]}
        for a in d["animations"]:
            for lid in a["layers"]:
                layer = layers.get(lid) or {}
                for kid in layer.get("keyframes", []):
                    if "replaceCharacter" in (kf.get(kid, {}).get("code") or ""):
                        out.add(a["name"])
    return out


def walk_script_speeds(ctx):
    """{animation: speed} for every x-speed a walk animation's own frame scripts set."""
    # the converter states them in its plan when it runs the source's clips itself
    planned = (ctx.get("plan") or {}).get("walk_script_speeds")
    if planned is not None:
        return dict(planned)
    out = {}
    # a speed a script states is in the source's units and crosses the boundary through `__sv`,
    # whose factor the fighter's own script defines; the figure held against the engine's
    # record is the one the engine sees
    vs = 1.0
    for sp in glob.glob(os.path.join(ctx["dir"], "library", "scripts", "*", "Script.hx")):
        m = re.search(r"function __sv\(v\) \{ return v \* ([\d.]+); \}", open(sp, encoding="utf8").read())
        if m:
            vs = float(m.group(1))
            break
    for p in glob.glob(os.path.join(ctx["dir"], "library", "entities", f"{ctx['id']}.entity")):
        d = json.load(open(p, encoding="utf8"))
        kf = {k["$id"]: k for k in d["keyframes"]}
        for a in d["animations"]:
            if not a["name"].startswith("walk"):
                continue
            for lid in a["layers"]:
                layer = next((l for l in d["layers"] if l["$id"] == lid), None)
                for kid in (layer or {}).get("keyframes", []):
                    code = kf.get(kid, {}).get("code", "") or ""
                    for wrapped, m in re.findall(r"setXSpeed\(\s*(__svx?\()?\s*(-?[\d.]+)\s*\)", code):
                        if float(m) > 0:
                            out[a["name"]] = round(float(m) * (vs if wrapped else 1.0), 4)
    return out


def floor_spot(eng):
    """stand the Fraymakers fighter in the middle of the stage's floor and answer the height it
    stands at there, off its own record: the spawn is a platform on the stage the gates run on,
    and the middle of the stage at the spawn's height is the gap between platforms."""
    eng.tell("reset")
    standing(eng, 2)
    eng.tell("e match.getCharacters()[0].setX(0)")
    standing(eng, 2)
    try:
        return float(eng.eval("match.getCharacters()[0].getY()"))
    except (TypeError, ValueError):
        return None


def c_physics(ctx, r):
    eng = ctx["engine"]
    stats = declared_stats(ctx)
    ctx["stats"] = stats
    r.evidence["declared"] = {k: stats[k] for k in ("walkSpeedCap", "dashSpeed", "runSpeed", "gravity",
                                                    "jumpSpeed", "friction") if k in stats}
    # hold a direction, let the fighter get up to speed, then measure over frames the script
    # itself counted. the declared cap it settles at is the one it declares for that gait.
    # frame count and position in ONE read, so the speed is a ratio of the same instant: read
    # apart they are a round trip out of step, and the ratio wanders by a tenth
    both = PULSE + ' + "|" + match.getCharacters()[0].getX()'
    # every declared gait reaches its cap within twenty frames or so, and a fast fighter held
    # for two seconds walks to the ledge and stops there (link read 0.33/frame on that
    # account), so: a short run-up, a short window, and the fighter stays on the floor
    # and the engine's own record of what animation the fighter was in while held, so a speed
    # that is off can be read against what it was doing (walking, turning, already dashing)
    # from the middle of the floor: the spawn is a platform on the stages the gates run on,
    # and a walk held from there runs off its edge and reads as a fall
    floor_spot(eng)
    eng.tell("record")
    eng.tell("hold right")
    # the run-up and the stretch to measure, counted by the engine; two samples of the
    # script's own frame counter bracket it as evidence that the fighter's script ran
    eng.tick(36)
    series = [tuple(float(v) for v in eng.eval(both).split("|"))]
    eng.tick(36)
    series.append(tuple(float(v) for v in eng.eval(both).split("|")))
    mark_r = eng.tell("release")
    eng.settle(mark_r, timeout=4.0)
    # the speed comes off the engine's own per-frame record of the held stretch: the longest
    # run of one moving animation, its first frames (the run-up) left out. the samples above
    # stand in only when the record is too short, since a sample taken before the input has
    # reached the fighter averages a standing stretch into the walk
    rows = raw_trace_rows(eng)
    r.evidence["animations_while_held"] = rle([lab for lab, _, _ in rows])[:8]
    r.evidence["series"] = [[round(p), round(x, 1)] for p, x in series]
    (p1, x1), (p2, x2) = series[0], series[-1]
    run = longest_moving_run(rows)
    if run is not None and run[1] - run[0] >= 8:
        i, j = run[0] + 3, run[1]
        v = (rows[j][1] - rows[i][1]) / (j - i)
        r.evidence["measured_from"] = f"{rows[i][0]} frames {i}-{j} of the record"
    elif p2 > p1:
        v = (x2 - x1) / (p2 - p1)
        r.evidence["measured_from"] = "sampled positions"
    else:
        r.fail("no frames ran while the direction was held")
        return
    r.evidence["measured_speed"] = round(v, 3)
    r.evidence["over_frames"] = (run[1] - run[0] - 3) if run is not None and run[1] - run[0] >= 8 else p2 - p1
    candidates = {k: stats[k] for k in ("runSpeed", "dashSpeed", "walkSpeedCap") if k in stats}
    # a fighter whose walk is driven by its own frame scripts declares its speed THERE, not in the
    # stats: sandbag's walk-in sets 3.25 a frame and walks at that in both engines (SSF2 measured
    # 3.67 a source frame, which scales to 3.49), while its stats carry the dash figure
    for k, s in sorted(walk_script_speeds(ctx).items()):
        candidates[f"walk script {k}"] = s
    if not candidates:
        r.skip("the fighter declares no ground speed to hold it to")
        return
    hit = [k for k, s in candidates.items() if s and abs(abs(v) - s) / s <= 0.08]
    ctx["walk_fm"] = abs(v)
    # the walk the cross-engine gate compares, measured here the way the source's will be:
    # fifteen source frames from a stand, off the engine's own record
    ctx["walk_fm_seq"] = walk_speed(eng, 2)
    scripted = walk_script_speeds(ctx)
    if hit:
        r.note(f"held-right speed {abs(v):.2f}/frame = declared {hit[0]} {candidates[hit[0]]}")
    elif scripted:
        # a walk its own frame scripts drive (sandbag's is a hop: one speed in, another a few
        # frames later, then none) averages to no single figure it declares; the source's own
        # walk, measured the same way, is the figure to hold it to (C6)
        r.note(f"held-right speed {abs(v):.2f}/frame; the walk is driven by its own frame scripts "
               f"{scripted}, so the cross-engine gate holds it to the source's own walk")
    else:
        r.fail(f"held-right speed {v:.2f}/frame matches none of the declared gaits {candidates}")
    # the walk is measured at the engine's own rate: a few seconds, and at several frames a tick
    # each round trip is a few dozen frames of walking, which reaches the ledge. from here on the
    # frame-paced gates can run faster
    eng.faster(ctx["opts"].get("speed", 1))


def _snaps_to_floor(eng, home_y):
    """True if the fighter re-attaches to the floor the instant it is set well above it (its set-Y
    does not take), so an aerial or recovery cannot be staged from the air. One test read; the next
    move re-parks the fighter itself, so this leaves nothing to clean up."""
    if home_y is None:
        return False
    try:
        eng.eval(f"match.getCharacters()[0].setY({home_y - 260:.1f}); "
                 "match.getCharacters()[0].setYVelocity(1); match.getCharacters()[0].toState(CState.FALL)")
        eng.tick(1)
        return bool(eng.eval("match.getCharacters()[0].isOnFloor()"))
    except Exception:
        return False


def c_moves(ctx, r):
    eng = ctx["engine"]
    fm = ctx.get("fm_lengths", {})
    # a park and its move go out in one batch and land a frame apart whatever the engine's
    # rate, and every wait is on the engine's own announcements, so the gate runs at speed
    eng.faster(ctx["opts"].get("speed", 1))
    played, missing, wrong, never_returned, early, traces = [], [], [], [], [], {}
    # moves the gate could not stage as intended (an aerial on a fighter that re-attaches to the
    # floor the instant it is parked above it): reported, not failed, since the move plays from a
    # real jump and the gate just cannot air-park a floor-snapping fighter
    unstaged = []
    phases = {}
    # where the fighter stands: read once it is standing still on its own record. a reset
    # forces the standing pose wherever the fighter is, in the air included, and a move parked
    # relative to a fighter left floating by the last one never comes down
    home_y = ctx.get("home_y")
    if home_y is None:
        home_y = floor_spot(eng)
    for mv in MOVES:
        if mv not in fm:
            continue  # the fighter was not converted with this move
        if eng.crashed():
            # the previous move took the engine down: nothing after it can play, and waiting
            # out every later move's timeouts is a quarter hour of dead air
            r.fail(f"the engine crashed during {played[-1] if played else 'the last move'}" + eng.crash_reason())
            break
        t_mv = time.time()
        ph = phases[mv] = {}
        if home_y is not None:
            # the record first: a fighter the last reset left standing on its floor is left
            # alone; one left elsewhere (in the air, on a platform) is set down and waited for
            eng.tell("record")
            eng.tick(4)
            rows = raw_trace_rows(eng)
            there = len(rows) >= 2 and all(r[0] in ("stand", "idle") and abs(r[2] - home_y) < 0.5 for r in rows[-2:])
            ph["there"] = there
            if not there:
                ph["rows"] = rows[-2:]
                # two statements: the `e` line takes them as such (eval would wrap them as one)
                eng.reply(eng.tell(f"e match.getCharacters()[0].setX(0); match.getCharacters()[0].setY({home_y - 2:.1f}); match.getCharacters()[0].setYVelocity(1); match.getCharacters()[0].toState(CState.FALL)"), "E:", 5)
                standing(eng, 2)
        # an aerial is measured in the air, where it belongs: parked high enough that the whole
        # animation plays out before the floor arrives, or it lands part-way through and reads
        # as a move cut short
        airborne = mv.startswith("aerial_") or mv == "special_up"
        # a move that travels (a dash attack at a fast fighter's speed, a recovery that drifts
        # forward) starts from the middle of the floor, or it carries the fighter off the ledge
        # from where it spawns and a move that ends in the blast zone reads as one that never
        # ended. an aerial stays over the spawn point: the middle of a stage can have a platform
        # under it, and a fighter parked above one lands on it part-way through the move
        if not airborne:
            eng.eval("match.getCharacters()[0].setX(0)")
        elif ctx.get("home_x") is not None:
            # over where it spawned, which is clear of platforms: the last grounded move left
            # it in the middle
            eng.eval(f"match.getCharacters()[0].setX({ctx['home_x']:.1f})")
        # the engine records every frame itself, so the length a move played is read off its
        # own trace and never off when the host happened to look. an aerial is parked and
        # launched in ONE batch: the park and the move a frame apart, not a round trip apart,
        # which is a dozen frames of falling each
        batch = ["record"]
        if airborne:
            g = ctx.get("stats", {}).get("gravity", 0.5) or 0.5
            # an up special is parked as high as any other move: a long one (donkey kong's runs
            # 118 frames) fell 700 px into the floor before it ended, and the blast-zone check
            # below catches a park that was too high
            cap = 1000.0
            h = min(cap, 0.5 * g * (1.5 * (fm[mv] + 12)) ** 2 + 80)
            # the fighter may have died on the previous move and not read a height back (getY is
            # None): fall back to the floor home so the park still happens and the gate does not
            # crash on float(None) (simon's C5)
            try:
                y = float(eng.eval("match.getCharacters()[0].getY()"))
            except (TypeError, ValueError):
                y = home_y if home_y is not None else 0.0
            batch.append(f"e match.getCharacters()[0].setY({y - h:.0f})")
        batch.append(f"move {mv}")
        mark = eng.tell_batch(batch)
        ph["park"] = round(time.time() - t_mv, 2); t_mv = time.time()
        if eng.reply(mark, "MOVE:", 10) is None:
            if airborne and _snaps_to_floor(eng, home_y):
                unstaged.append(f"{mv}: not staged (fighter snaps back to the floor; aerial can't be air-parked)")
            else:
                missing.append(f"{mv}: no dispatch")
            continue
        ph["dispatch"] = round(time.time() - t_mv, 2); t_mv = time.time()
        # until the move's animation ends, then until the fighter is standing again (a recovery
        # special falls first). each await returns when the animation changes OR loops, and a
        # fall loops every few frames, so the wait is bounded by the fighter's own frame count
        # and not by a number of awaits
        # the move is over when the fighter is back in a pose a move comes back to, and the gate
        # wants it standing (a recovery special falls first, then lands); both are read off the
        # engine's own animation announcements, no round trip spent, and bounded in seconds
        # rather than by a count of awaits
        eng.settle(mark, timeout=20.0)
        ph["settle"] = round(time.time() - t_mv, 2); t_mv = time.time()
        stood = eng.await_pose(("stand", "idle"), timeout=20.0) < 20.0
        ph["stand"] = round(time.time() - t_mv, 2); t_mv = time.time()
        # the announcement of a pose runs a frame ahead of the record that carries it: one more
        # engine frame, so the trace read next ends in the pose the engine announced
        eng.tick(1)
        mark_t = eng.tell("trace")
        runs = trace_runs(eng.read_trace(mark_t))
        ph["trace"] = round(time.time() - t_mv, 2)
        if stood and runs and runs[-1][0].lower() not in ("stand", "idle"):
            runs = runs + [("stand", 1)]
        traces[mv] = runs[:8]
        # the move's OWN run, by its exact name: what follows it (a loop state, an ending, a
        # landing) is the next state, and counts as that
        # a special's air variant is the same move: launched airborne it plays that one, and a
        # special that hops off the ground on its first frame (sonic's) is in it at once
        names = {mv.upper()} | ({mv.upper() + "_AIR"} if airborne or mv.startswith("special_") else set())
        idx = next((i for i, (a, _) in enumerate(runs) if a.upper() in names), None)
        died = next((i for i, (a, _) in enumerate(runs) if a.upper() == "REVIVAL"), None)
        if idx is None:
            if died is not None:
                missing.append(f"{mv}: the fighter died before the move played (parked into the blast zone?)")
            elif airborne and _snaps_to_floor(eng, home_y):
                # the fighter re-attaches to the floor the instant it is set above it, so its aerial
                # could not be staged in the air here. the move plays from a real jump (a gate can't
                # air-park a floor-snapping fighter), so report it, do not fail on it.
                unstaged.append(f"{mv}: not staged (fighter snaps back to the floor; aerial can't be air-parked)")
            else:
                missing.append(f"{mv}: the move's animation never played (saw {[a for a, _ in runs][:5]})")
        else:
            label, seen = runs[idx]
            want = fm.get(label.lower(), fm[mv])
            after = runs[idx + 1][0].lower() if idx + 1 < len(runs) else ""
            if abs(seen - want) <= 1:
                played.append(mv)
            elif seen > want + 1:
                # its own code kept it past the animation's end, into a normal state after: a dash
                # attack that ends when the slide stops does this in the source too. whether the
                # source keeps it the same number of frames is the cross-engine question (C6).
                # a move that never ends at all is caught below, not here
                early.append(f"{mv}: kept by its own code {seen - want} frames past its emitted {want}, into {after}")
                played.append(mv)
            elif airborne and after.startswith("land") and want <= 150:
                # not the move's doing: the floor arrived first, so the gate parked it too low
                missing.append(f"{mv}: cut by landing at {seen} of {want} frames (parked too low)")
            elif airborne and after.startswith("land"):
                # a move longer than any fall (a charge, a flight) ends on the floor by nature;
                # its own length is the cross-engine gate's question
                early.append(f"{mv}: {seen} of {want} frames before the floor (a move longer than a fall)")
                played.append(mv)
            else:
                # its own code ended it early, into another state. whether the source does the
                # same at the same frame is the cross-engine question (C6), not a length failure
                early.append(f"{mv}: ended by its own code at {seen} of {want} frames, into {after}")
                played.append(mv)
            if died is not None and died < idx:
                missing.append(f"{mv}: the fighter died before the move played")
        if not runs or runs[-1][0].upper() != "STAND":
            never_returned.append(f"{mv}: ended in {runs[-1][0] if runs else '?'}")
        # every move starts from the same place: standing, on the ground, where it spawned. the
        # reset puts the fighter at the spawn point, which is above the floor, so the next move
        # is not sent until the landing is over (sonic's neutral special fired airborne and
        # played its air variant)
        # and with nothing the move left behind still live: a projectile that outlasts its move goes
        # off under the next one (samus's bomb, set by her down special, blew her into the air as
        # her grab was pressed). waited out on the engine's frames, a bounded number of them
        for _ in range(30):
            if not eng.n_projectiles():
                break
            eng.tick(8)
        mark_r = eng.tell("reset")
        eng.settle(mark_r, timeout=6.0)
        eng.await_pose(("stand", "idle"), timeout=6.0)
        ph["reset"] = round(time.time() - t_mv, 2)
    r.evidence = {"home_y": home_y, "phases": phases, "played": played, "wrong_length": wrong[:10], "missing": missing[:10],
                  "never_returned": never_returned[:10], "length_chosen_by_own_code": early[:10], "unstaged": unstaged[:10],
                  "traces": {m: traces[m] for m in traces if any(m in w for w in wrong + missing + never_returned)}}
    if unstaged:
        r.note(f"{len(unstaged)} move(s) not staged (aerial on a floor-snapping fighter; plays from a real jump): "
               + ", ".join(n.split(":")[0] for n in unstaged))
    if missing:
        r.fail(f"{len(missing)} move(s) did not play: {missing[0]}")
    if wrong:
        r.fail(f"{len(wrong)} move(s) ran the wrong length: {wrong[0]}")
    held = held_moves(ctx)
    held_out = [n for n in never_returned if n.split(":")[0] in held]
    never_returned = [n for n in never_returned if n.split(":")[0] not in held]
    if held_out:
        r.note(f"{len(held_out)} move(s) held by their own code until an input ends them (C6 decides): "
               + ", ".join(n.split(":")[0] for n in held_out))
    if never_returned:
        r.fail(f"{len(never_returned)} move(s) never returned to STAND: {never_returned[0]}")
    # a move that plays to the end and comes back can still have thrown on every frame (an
    # animation it asks for by a name the entity does not have): what the engine reported while
    # the moves ran is part of the verdict, one line per distinct complaint
    errs = eng.errors()
    distinct = list(dict.fromkeys(e.split("(origin", 1)[0].strip() for e in errs))
    r.evidence["script_errors"] = distinct[:8]
    if errs:
        r.fail(f"{len(errs)} script error line(s) while the moves ran ({len(distinct)} distinct): {distinct[0][:110]}")
    eng.faster(ctx["opts"].get("speed", 1))
    if r.status == "PASS":
        r.note(f"{len(played)} moves played and came back"
               + (f", {len(early)} ran a length their own code chose (C6 decides)" if early else ""))


# where the other fighter stands during the input gate, in source units right of home: out of
# a jab's reach on either engine (a hit's stop would lengthen the move on one side only; the
# heaviest jab in the sample reaches 150) and within a homing attack's, on the floor of the
# plainest stage. the Fraymakers side stands it the same distance in ITS units
OTHER_AT = 250.0
# frames a parked recovery falls on the port before its input goes in (one command a frame)
RECOVERY_LEAD = 4
# a recovery's fall before its input, in source frames, on both engines (a step of the seq)
RECOVERY_FALL = 3
# the stage under a character run, named on both engines: the source's battlefield and the port
# of that same stage (a parachute that lands on the source's top platform at x 0 fell to the
# floor on this engine's default stage, and read as a move running three times as long)
CHAR_STAGE_SSF2 = "battlefield"
CHAR_STAGE_FM = "battlefieldssf2"
# a distance on the port is the source's times the size scale (the stats' size multiplier):
# the other fighter OTHER_AT away in the source's pixels stands OTHER_AT * SIZE_SCALE away here,
# or a dash that meets it on one engine misses on the other
SIZE_SCALE = 1.3


def engine_scale(ctx):
    """the converter's own velocity scale for this fighter (source units per frame to the
    port's), asked of it; the spatial scale is that times the frame-rate ratio."""
    scale = VELOCITY_SCALE
    try:
        _, sout, _ = run([BIN, "ssf2", "scale", ctx["ssf"], ctx["id"]], 120)
        m = re.search(r"velocity_scale=([\d.]+)", sout)
        if m:
            scale = float(m.group(1))
    except Exception:
        pass
    return scale


# each input starts at the same moment of its match on both engines, counted in source frames
# from the stand the match came up in: what a fighter charges over time (wario's waft, a charge
# shot) is charged the same on both. the lead covers what the source's side does before its
# inputs (the walk), the span the longest input plus its settle
PACE_LEAD = 240
PACE_SPAN = 240


def drive_inputs(eng, inputs, settle, frames_per_source_frame=2, other_at=OTHER_AT, home_y=None, base=None):
    """{input: [(label, frames), ...]} from the engine's own per-frame trace, one input at a
    time from a neutral stand. a tap is one SOURCE frame long on either engine: two frames
    here, one on the engine that runs at the source's rate. `base` is the engine's clock at
    the stand its match came up in; with it each input starts on the shared schedule."""
    out = {}
    eng.late = []
    # where the fighter starts: a special that carries it off the stage leaves the next input
    # to a fighter that is falling, or dead, and the engine's reset does not put it back
    home = None
    if isinstance(eng, Ssf2Engine):
        def read(expr):
            # the answer is the first reply after the session's echo of this command: a reply
            # to an earlier command can land in the log after this one was sent
            return ssf2_read(eng, expr)
        try:
            # the middle of the floor, at the height the fighter comes to rest at: it is spawned
            # in the air, announces STAND while its entrance still holds it up, and drops to
            # the floor on the first reset, so one reset goes first and its height is then
            # read until it stops changing. put back at its spawn it can stand at a ledge,
            # where a down input leans over it instead of starting a move
            # the entrance holds the fighter up until its height is written: written back where
            # it is, it drops, lands, and stands where it will stand from now on. the height it
            # then stands at comes off its own per-frame record, since the reflection read of
            # its height keeps answering the spawn's
            y = read("e p0.Y")
            eng.tell(f"e p0.Y = {y:.2f}")
            eng.tell("reset")
            # and in the middle of the stage: the spawn is a platform, the middle the floor
            eng.tell("e p0.X = 0")
            eng.settle(len(eng.log()), timeout=3.0)
            eng.await_pose(timeout=4.0)
            eng.tell("record")
            eng.tick(6)
            ends = re.findall(r"end=\(([-\d.]+),([-\d.]+)\)", eng.read_trace(eng.tell("trace")))
            if ends:
                y = float(ends[-1][1])
            home = (0.0, y)
        except Exception as e:
            print(f"[drive] the SSF2 home could not be read ({e!r}); inputs run from the spawn", file=sys.stderr)
            home = None
    elif home_y is not None:
        home = (0.0, home_y)
    else:
        try:
            # standing still on its own record first: a reset forces the standing pose on a
            # fighter the last gate left in the air, and its height there is not the floor's
            y = floor_spot(eng)
            home = (0.0, y) if y is not None else None
        except Exception:
            home = None
    eng.home = home
    eng.timings = {}
    # inputs the gate could not drive as intended (a recovery on a fighter that re-attaches to the
    # floor the instant it is set above it, so it cannot be launched from the air): reported, not
    # failed, since the move itself is verified from a real jump, the gate just cannot stage it here
    eng._undriveable = set()
    for n, inp in enumerate(inputs):
        t_in = time.time()
        pre_sent = None
        lead_step = 0
        if base is not None:
            want = (PACE_LEAD + n * PACE_SPAN) * frames_per_source_frame
            now = eng.clock()
            if now is not None:
                if now - base < want:
                    eng.tick(want - (now - base))
                else:
                    eng.late.append(f"{inp}: {(now - base) // frames_per_source_frame} source frames in, "
                                    f"past its slot at {want // frames_per_source_frame}")
        # drop anything the last input left held, or its direction leaks into this one: the input
        # before the up special is a side special (right held), and a right still down turns the
        # recovery into a side special first (goku read special_side_air before special_up_air)
        eng.tell("release all")
        # a recovery (the up special) is the move a fighter uses in the AIR, and dispatched from the
        # ground it rises and lands before it ends -- so the source's grounded run is cut by the
        # floor (heavyland) while the port rises clear, and the two lengths are not the same fact.
        # measure it where it belongs: parked high, falling, so the whole rise plays out on both
        # engines before either meets the floor. (C5 parks its special_up airborne for the same reason.)
        recovery = "up+special" in inp
        # two spots on the floor, taken in turn: what the last input left behind where it was
        # used (sonic's spring, which bounces whoever stands on it) is not under the next one
        at = (home[0] + (0.0 if n % 2 == 0 else -other_at * 0.4)) if home is not None else 0.0
        # how high to park a recovery: enough airtime for the rise to finish before the floor, but
        # clear of the top blast zone. the rise adds its own height on top of this, so a moderate
        # lift is plenty, and the source and port need only each have room, not the same room.
        air = 260.0
        # the other fighter stands the same distance away on both engines, so a move that
        # seeks it meets it at the same moment
        # one batch: the session takes them in order, and each is a round trip on its own
        if home is not None and isinstance(eng, Ssf2Engine):
            if recovery:
                # standing first, as on the port: a fighter the last input left hanging from a ledge
                # stays hanging when it is set elsewhere (waluigi's recovery started from a hang)
                eng.tell_batch([f"e p0.X = {at:.2f}", f"e p0.Y = {home[1]:.2f}", "reset"])
                standing(eng, frames_per_source_frame)
                # set it high with a little downward speed and no reset (reset would stand it on the
                # floor); it falls, and the up special fires from the air the way it is meant to
                # parked, recorded and pressed in ONE batch, the fall before the input a step of the
                # seq itself: the fall is then counted in the engine's frames from the park, not
                # in round trips (which vary with load, and a float's landing moved with them)
                pre_sent = eng.tell_batch([f"e p1.X = {at + other_at:.2f}", f"e p1.Y = {home[1]:.2f}",
                                           f"e p0.X = {at:.2f}", "e p0.faceRight()", "e p1.faceLeft()", f"e p0.Y = {home[1] - air:.2f}",
                                           "record", f"seq none:{RECOVERY_FALL} {fm_steps(inp, frames_per_source_frame, True)}"])
                lead_step = 1
            else:
                # put back first, then reset: the reset stands the fighter up, and standing where it
                # was put keeps it from starting the next input a few frames into a fall
                # facing the other fighter on both engines: the source respawns facing the middle
                # of the stage, this engine's spawn faces the other way, and a move that travels
                # met the other fighter on one engine and the ledge on the other. the other one faces
                # back on both: it stood facing away on the source and toward on this engine, and a
                # hurtbox that is not the same both ways put it a few pixels nearer one kick than the
                # other (falcon's down special met it on its last active frame here only)
                eng.tell_batch([f"e p1.X = {at + other_at:.2f}", f"e p1.Y = {home[1]:.2f}",
                                f"e p0.X = {at:.2f}", f"e p0.Y = {home[1]:.2f}", "reset", "e p0.faceRight()", "e p1.faceLeft()"])
        elif home is not None:
            at_fm = at * SIZE_SCALE
            park_y = (home[1] - air * SIZE_SCALE) if recovery else (home[1] - 2)
            park = [f"e match.getCharacters()[1].setX({at_fm + other_at * SIZE_SCALE:.1f}); match.getCharacters()[1].setY({home[1]:.1f}); match.getCharacters()[1].faceLeft()",
                    "reset",
                    # from above the floor, falling: a fighter set on the floor's
                    # height is not attached to it and takes no input; landed by the
                    # engine it is. a recovery is set far higher so its rise finishes airborne
                    # a recovery falls from rest, as the source's park does (at rest in the air is
                    # a step of gravity here: this engine moves before it adds gravity, the source
                    # after); a grounded input is nudged the two pixels onto the floor
                    f"e match.getCharacters()[0].setX({at_fm:.1f}); match.getCharacters()[0].setY({park_y:.1f}); "
                    + (f"match.getCharacters()[0].setYVelocity(match.getCharacters()[0].getCharacterStat(\"gravity\") * 1.5); " if recovery else "match.getCharacters()[0].setYVelocity(1); ")
                    + "match.getCharacters()[0].toState(CState.FALL); "
                    + "match.getCharacters()[0].faceRight()"]
            if recovery:
                # standing on the floor first: a special used in the air stays used until the
                # fighter lands, and the input before this one can end airborne (pikachu's side
                # special does), which left its recovery refused
                eng.tell("reset")
                standing(eng, frames_per_source_frame)
                # parked and pressed in one batch at the engine's own rate, so the fall before the
                # input is the source's few frames and not however many a round trip takes at eight
                # frames a tick (mario went into his up special with seven frames of fall speed)
                if getattr(eng, "speed", 1) > 1:
                    eng._seq_speed = eng.speed
                    eng.reply(eng._raw_tell("speed 1"), "", 10)
                # a few frames falling before the input: pressed on the frame the fighter is put in
                # its fall, the engine is still taking the state and drops the special (luigi,
                # pikachu and ryu fell through their recoveries)
                pre_sent = eng.tell_batch(park + ["record", f"seq none:{RECOVERY_FALL * frames_per_source_frame} {fm_steps(inp, frames_per_source_frame)}"])
                lead_step = 1
            else:
                eng.tell_batch(park)
        else:
            eng.tell("reset")
        # standing on its own record before the input goes: a fighter the last input killed
        # is reviving, and one put back on the floor lands first. a recovery is dispatched from the
        # air on purpose, so it is NOT settled to a stand first -- it is left falling.
        if not recovery and not standing(eng, frames_per_source_frame):
            eng.tell("reset")
            eng.await_pose(timeout=4.0)
            standing(eng, frames_per_source_frame)
        # a grounded input MUST fire from the floor: on the port, a floaty fighter can still be a
        # frame off the floor when standing() gave up (the height was settling, not settled), and an
        # attack pressed a frame airborne comes out as the aerial (kirby: jab -> aerial_neutral). so
        # before a non-recovery input on the port, make the floor certain -- set it back on the floor
        # and let the engine attach it -- rather than trust the pose. a fighter already grounded is
        # unaffected (it is put where it already is). recoveries are meant to fire from the air, so
        # they are left alone.
        if not recovery and home is not None and not isinstance(eng, Ssf2Engine):
            for _ in range(6):
                try:
                    if eng.eval("match.getCharacters()[0].isOnFloor()"):
                        break
                except Exception:
                    break
                eng.tell(f"e match.getCharacters()[0].setX({at * SIZE_SCALE:.1f}); "
                         f"match.getCharacters()[0].setY({home[1] - 2:.1f}); "
                         "match.getCharacters()[0].setYVelocity(1); match.getCharacters()[0].toState(CState.FALL)")
                eng.tick(2)
            standing(eng, frames_per_source_frame)
        steps = fm_steps(inp, frames_per_source_frame, isinstance(eng, Ssf2Engine))
        # where the other fighter stands as the input goes, and in what: a move that meets it
        # branches on it, and one the last input left hurt or moved is not where it was put
        if not hasattr(eng, "others"):
            eng.others = {}
        try:
            if isinstance(eng, Ssf2Engine):
                eng.others[inp] = [ssf2_read(eng, "e p1.X") - (home[0] if home else 0.0), None]
            else:
                o = eng.eval("match.getCharacters()[1].getX() + ' ' + match.getCharacters()[1].getAnimation()")
                x, anim = (o or " ").split(" ", 1)
                eng.others[inp] = [float(x) / SIZE_SCALE - (home[0] if home else 0.0), anim]
        except Exception:
            pass
        if pre_sent is not None:
            mark_s = pre_sent
        else:
            eng.tell("record")
            mark_s = eng.tell(f"seq {steps}")
        # every step taken and acknowledged first: judged before that, an input the engine has
        # not reached yet looks like one that did nothing
        eng.await_seq(mark_s, len(inp.split()) + lead_step)
        # the move is over when the fighter is back in a pose a move comes back to; a fixed
        # wait sized for the longest move spent most of its time on the shortest
        eng.settle(mark_s, timeout=max(settle * 4, 6.0))
        # this engine announces every fighter's changes on one feed, so the other fighter coming
        # back to standing can end the wait while the driven one's move has only begun (mario's
        # up tilt read as one frame long): its own animation is asked, a few frames apart, until
        # it is back in a pose a move comes back to
        if not isinstance(eng, Ssf2Engine):
            for _ in range(60):
                try:
                    a = str(eng.eval("match.getCharacters()[0].getAnimation()") or "").split(":")[-1].strip().lower()
                except Exception:
                    break
                if not a or eng.is_terminal(a):
                    break
                eng.tick(4 * frames_per_source_frame)
        mark_t = eng.tell("trace")
        out[inp] = trace_runs(eng.read_trace(mark_t))
        # ...and where the fighter was on every frame of it, for the path the move takes
        if not hasattr(eng, "paths"):
            eng.paths = {}
        eng.paths[inp] = raw_trace_rows(eng)
        # some fighters re-attach to the floor the instant they are set above it (their set-Y
        # does not take), so the recovery cannot be staged: read off its own record, a recovery
        # that starts on the floor is marked un-driveable rather than read as the move
        if recovery and not isinstance(eng, Ssf2Engine):
            first = next((r for r in eng.paths[inp] if not r[0].lower().startswith(("fall", "stand", "idle"))), None)
            head = eng.paths[inp][:4]
            if head and all(r[0].lower().startswith(("stand", "idle", "land")) for r in head) and first is not None:
                eng._undriveable.add(inp)
        # a directional recovery (goku's teleport) reads its heading only if up is cleanly down as
        # special fires, which is timing-sensitive and sometimes plays the side special instead. if a
        # recovery came out as a non-up special on the port, re-drive it (re-park in the air, re-seq)
        # up to twice; the move is right, the stage read the heading late. only for a port recovery
        # that actually spawned a wrong special, so a correct one never re-drives.
        if recovery and not isinstance(eng, Ssf2Engine) and inp not in eng._undriveable:
            for _ in range(2):
                mv = next((a.lower() for a, _ in out[inp]
                           if not a.lower().startswith(("stand", "idle", "fall", "land"))), None)
                if mv is None or "special_up" in mv:
                    break  # played the up special (or nothing to judge): keep it
                eng.tell("release all")
                if getattr(eng, "speed", 1) > 1:
                    eng._seq_speed = eng.speed
                    eng.reply(eng._raw_tell("speed 1"), "", 10)
                mark_s = park_then_press(eng, park, steps)
                eng.await_seq(mark_s, len(inp.split()))
                eng.settle(mark_s, timeout=max(settle * 4, 6.0))
                out[inp] = trace_runs(eng.read_trace(eng.tell("trace")))
                eng.paths[inp] = raw_trace_rows(eng)
        eng.timings[inp] = round(time.time() - t_in, 1)
    return out


def park_then_press(eng, park, steps):
    """the park commands, a few frames of fall, then the record and the input. the session takes
    queued commands one a frame but starts a record the moment it reads one, so the record waits
    on a tagged answer from the end of the park or it catches the last input's leftovers"""
    eng.n = getattr(eng, "n", 0) + 1
    tag = f"park{eng.n}:"
    # one engine frame more than the lead: the source's park falls that half source frame longer
    # before the input (measured on the c6-paths records: a float that lands off its start height
    # read the difference as length)
    mark = eng.tell_batch(park + ["e 0"] * (RECOVERY_LEAD + 1) + [f'e "{tag}" + 1'])
    eng.reply(mark, "E:" + tag, 10)
    eng.tell("record")
    return eng.tell(f"seq {steps}")


def fm_steps(inp, per, source=False):
    """`a:10 b` -> hold a for 10 source frames, then b for one, in this engine's frames. a TAPPED
    jump is a short hop on the source; the port decides the hop from input the injected mask
    cannot release early, so the tap names the short hop (see `shorthop`)"""
    tokens = inp.split()
    if not source:
        tokens = ["jump+shorthop" if t == "jump" else t for t in tokens]
    return " ".join(f"{t.split(':')[0]}:{int(t.split(':')[1]) * per}" if ":" in t else f"{t}:{per}" for t in tokens)


def standing(eng, per_source=1):
    """block until the fighter's own per-frame record shows it standing still for a few
    frames: a reset forces the standing animation on a fighter still in the air (it lands a
    moment later), so the pose alone is not enough; its height has to have stopped changing."""
    for _ in range(12):
        eng.tell("record")
        eng.tick(4 * per_source)
        rows = raw_trace_rows(eng)
        if len(rows) >= 3 and all(r[0] in ("stand", "idle") for r in rows[-3:]) \
                and max(r[2] for r in rows[-3:]) - min(r[2] for r in rows[-3:]) < 0.01:
            return True
    return False


NEUTRAL_LABELS = ("stand", "idle", "fall", "land", "walk", "crouch")


def move_path(rows, per_source, scale):
    """[(dx, dy)] per SOURCE frame of the move a recorded input played, from the frame the move
    began, in source px: a port row is divided by the size scale and every second frame taken."""
    start = next((i for i, (a, _, _) in enumerate(rows) if not a.startswith(NEUTRAL_LABELS)), None)
    if start is None:
        return []
    x0, y0 = rows[start][1], rows[start][2]
    out = []
    for i in range(start, len(rows), per_source):
        a, x, y = rows[i]
        if a.startswith(("stand", "idle")) and out:
            break
        out.append(((x - x0) / scale, (y - y0) / scale))
    return out


def compare_paths(src, port, floor_px=12.0, share=0.15):
    """the worst gap between two move paths over the frames both have, against a tolerance of
    `floor_px` or `share` of the source's own reach, whichever is larger."""
    n = min(len(src), len(port))
    if n < 3:
        return None
    reach = max(max(abs(dx), abs(dy)) for dx, dy in src[:n]) if n else 0.0
    tol = max(floor_px, share * reach)
    worst, at, axis = 0.0, 0, "y"
    for i in range(n):
        for k, name in ((0, "x"), (1, "y")):
            d = abs(src[i][k] - port[i][k])
            if d > worst:
                worst, at, axis = d, i, name
    return {"frames": n, "reach": round(reach, 1), "tolerance": round(tol, 1), "worst": round(worst, 1),
            "at_frame": at, "axis": axis, "within": worst <= tol,
            "src_end": [round(v, 1) for v in src[n - 1]], "port_end": [round(v, 1) for v in port[n - 1]]}


def raw_trace_rows(eng):
    """[(animation, x)] per frame of the record window, off the engine's own raw trace."""
    mark_t = eng.tell("trace raw")
    head = eng.reply(mark_t, "TRACERAW:", 20)
    if head is None:
        return []
    want = int(re.search(r"(\d+) frames", head).group(1)) if re.search(r"(\d+) frames", head) else 0
    # the rows follow the header; the ones not in the log yet arrive on the feed
    def have():
        return sum(1 for line in eng.log()[mark_t:].splitlines() if re.match(r"(?:<< )?\s*\d+\t", line))
    t0 = time.time()
    while have() < want and time.time() - t0 < 10:
        if eng.next_line(1.0) is None:
            break
    rows = []
    for line in eng.log()[mark_t:].splitlines():
        m = re.match(r"(?:<< )?\s*\d+\t(.*)", line)
        if not m:
            continue
        f = m.group(1).split("|")
        if len(f) < 4:
            continue
        try:
            rows.append((f[0].strip().lower(), float(f[2]), float(f[3])))
        except ValueError:
            continue
    return rows


def rle(labels):
    runs = []
    for lab in labels:
        if runs and runs[-1][0] == lab:
            runs[-1][1] += 1
        else:
            runs.append([lab, 1])
    return runs


def longest_moving_run(rows):
    """(first, last) row indexes of the longest run of one animation that is not a stand."""
    best, i = None, 0
    while i < len(rows):
        j = i
        while j + 1 < len(rows) and rows[j + 1][0] == rows[i][0]:
            j += 1
        if rows[i][0] not in ("stand", "idle") and (best is None or j - i > best[1] - best[0]):
            best = (i, j)
        i = j + 1
    return best


def walk_speed(eng, per_source):
    """the fighter's average speed over fifteen source frames of holding right from a stand,
    off the engine's own per-frame record: the same span on both engines, ramp included."""
    eng.tell("reset")
    # from the middle of the floor: the hold before this one left the fighter near a ledge,
    # and fifteen frames of walking from there end in a fall. on the source the reset puts
    # the fighter back at its spawn, in the air, so it is set down on the floor it stood on
    home = getattr(eng, "home", None)
    if isinstance(eng, Ssf2Engine):
        eng.tell("e p0.X = 0")
        if home is not None:
            eng.tell(f"e p0.Y = {home[1]:.2f}")
    else:
        eng.tell("e match.getCharacters()[0].setX(0)")
    eng.settle(len(eng.log()), timeout=3.0)
    eng.await_pose(timeout=4.0)
    # and standing on the record itself before the hold starts
    standing(eng, per_source)
    eng.tell("record")
    held = 15 * per_source
    mark_s = eng.tell(f"seq right:{held}")
    eng.await_seq(mark_s, 1)
    # the held stretch and a few frames after, counted by the engine
    eng.tick(held + 4)
    rows = raw_trace_rows(eng)
    # from the first frame of walking, on either engine's own names for it, over the held span
    i = next((k for k, (lab, _, _) in enumerate(rows) if lab.startswith("walk")), None)
    if i is None:
        return None
    j = min(i + held - 1, len(rows) - 1)
    if j - i < 4:
        return None
    # the speed is the frames' own steps, not the span end to end: a fighter put down by a
    # reset a moment before can still be sliding into place when the record opens, and one
    # teleport-sized step in the window read as a third more speed (zelda's source walk read
    # 5.33 a frame for a walk of 4.00). a step far beyond the median is that, and is left out.
    steps = [abs(rows[k + 1][1] - rows[k][1]) for k in range(i, j)]
    print(f"[walk] {type(eng).__name__} rows {i}..{j}: " + " ".join(f"{r[0]}:{r[1]:.1f}" for r in rows[max(0, i - 2):j + 1]), file=sys.stderr)
    raw = [l for l in eng.log()[mark_s:].splitlines() if re.match(r"(?:<< )?\s*\d+\t", l)]
    print("[walk-raw] " + " | ".join(l.strip()[:60] for l in raw[max(0, i - 1):i + 6]), file=sys.stderr)
    ordered = sorted(steps)
    median = ordered[len(ordered) // 2]
    kept = [d for d in steps if median <= 0 or d <= 3 * median]
    return sum(kept) / len(kept) if kept else 0.0


def c_ssf2(ctx, r):
    if not ctx["opts"].get("ssf2"):
        r.skip("not requested (--ssf2)")
        return
    # the fighter's own map from the source's animation names to the ones it was converted to
    stats = json.load(open(os.path.join(ctx["dir"], "conversion_stats.json"), encoding="utf8"))
    to_fm = stats.get("ssf2_to_fm_anim", {})
    # on a FRESH engine: the move gate has played every move once by now, and what a move
    # stores survives it (samus's charge shot keeps its charge between uses, and a tap after a
    # full charge fires it), while the source's side starts from a fresh match
    timing = {}
    t = time.time()
    # a new match in the running engine: its content is built afresh, so nothing a move
    # stored in the last match survives, and the app is not relaunched for it
    fresh = ctx["engine"]
    # on the stage the boot baked (the same one the source runs on); unnamed, the new match
    # would come up on the engine's default stage, where the floor the gates park on is air
    mark = fresh.tell(f"spawn {ctx['id']},{ctx['id']} {CHAR_STAGE_FM}")
    # a RELAUNCH tears the old match down and builds a new one, and a script that reads
    # match.characters during that window sees it momentarily null and logs a one-shot error
    # BEFORE the new match reports LAUNCHED. That error does not stop the launch (LAUNCHED follows
    # it every time), so this wait must not fail fast on it -- the launch either arrives or the
    # 60s runs out. Fail-fast here read the transient as a dead command and failed every character.
    if fresh.reply(mark, "LAUNCHED", 60, failfast=False) is None:
        r.fail("the engine never launched a new match for the inputs")
        return
    # the NEW match's intro, then its stand: the first match's are in the log too
    t0 = time.time()
    while "ANIM:INTRO" not in fresh.log()[mark:]:
        if time.time() - t0 > 60 or fresh.next_line(1.0) is None and time.time() - t0 > 60:
            break
    if "ANIM:INTRO" not in fresh.log()[mark:]:
        r.fail("the new match never started its intro")
        return
    if fresh.await_pose(("stand", "idle"), timeout=30.0) >= 30.0:
        r.fail("the new match never reached STAND")
        return
    # every input starts from a stand on the floor, so nothing here needs the engine's own
    # rate; the same frames pass several to a tick
    fresh.faster(ctx["opts"].get("speed", 1))
    fm_base = fresh.clock()
    timing["fm_relaunch"] = round(time.time() - t, 1); t = time.time()
    scale = engine_scale(ctx)
    timing["scale"] = round(time.time() - t, 1); t = time.time()
    fm = drive_inputs(fresh, C6_ACTIVE, 2.5, other_at=OTHER_AT, home_y=ctx.get("home_y"), base=fm_base)
    timing["fm_inputs"] = round(time.time() - t, 1)
    timing["fm_per_input"] = getattr(fresh, "timings", None)
    fm_undriveable = getattr(fresh, "_undriveable", set())
    t = time.time()
    fresh.close()
    ctx["engine"] = None
    timing["fm_close"] = round(time.time() - t, 1); t = time.time()
    r.evidence["timing"] = timing
    boot_thread = ctx["opts"].get("ssf2_boot")
    if boot_thread is not None:
        boot_thread.join()
    s = Ssf2Engine(f"ssf2-{ctx['id']}")
    ctx["ssf2"] = s
    # the same two-fighter roster the Fraymakers side gets: a move that seeks the nearest
    # fighter finds one on both engines, or neither
    if s.boot(["--char", f"{ctx['id']},{ctx['id']}", "--stage", CHAR_STAGE_SSF2]) is None:
        r.fail("the SSF2 session never came up")
        return
    if s.wait(("ANIM:",), 120) is None:
        r.fail("SSF2 never reached a match")
        return
    # the source at the same rate as the port: its frame rate is raised by the same factor
    s.faster(ctx["opts"].get("speed", 1))
    # the fighter's entrance eats the first input; it is over when a stand follows it
    t0 = time.time()
    while time.time() - t0 < 15:
        text = s.log()[s.mark0:]
        if "ANIM:ENTRANCE" in text and text.rfind("ANIM:STAND") > text.rfind("ANIM:ENTRANCE"):
            break
        s.next_line(1.0)
    ssf2_base = s.clock()
    timing["ssf2_boot"] = round(time.time() - t, 1); t = time.time()
    # the walk FIRST, held the same way on both engines for the same fifteen source frames from
    # a stand, each read off its own per-frame record: an average over a window no ledge cuts
    # short, and the same window for a walk that hops (sandbag's) as for one that does not.
    # before the inputs: a special can change who is walking (zelda's down special makes her
    # sheik in the source, and sheik's walk was held against zelda's)
    walk = walk_speed(s, 1)
    timing["ssf2_walk"] = round(time.time() - t, 1); t = time.time()
    ssf2 = drive_inputs(s, C6_ACTIVE, 1.3, frames_per_source_frame=1, base=ssf2_base)   # the source's own rate
    r.evidence["late"] = {"fraymakers": getattr(fresh, "late", []), "ssf2": s.late}
    timing["ssf2_inputs"] = round(time.time() - t, 1)
    timing["ssf2_per_input"] = getattr(s, "timings", None)
    t = time.time()
    s.close()
    timing["ssf2_close"] = round(time.time() - t, 1)
    # every input's per-frame record on both engines, whole, for reading a difference offline
    try:
        with open(os.path.join(ROOT, "build", "verify", f"c6-paths-{ctx['id']}.json"), "w", encoding="utf8") as fh:
            json.dump({"source": getattr(s, "paths", {}), "port": getattr(fresh, "paths", {}),
                       "other": {"source": getattr(s, "others", {}), "port": getattr(fresh, "others", {})}}, fh)
    except OSError:
        pass
    # THE PATH A MOVE TAKES, on both engines, from the frame it began: evidence per input
    paths = {}
    for inp in C6_ACTIVE:
        src_rows = getattr(s, "paths", {}).get(inp) or []
        fm_rows = getattr(fresh, "paths", {}).get(inp) or []
        cmp = compare_paths(move_path(src_rows, 1, 1.0), move_path(fm_rows, 2, SIZE_SCALE))
        if cmp is not None:
            if not cmp.get("within"):
                # the rows a path was read from, so a divergence can be read without driving it again
                cmp["rows"] = {"source": [[r[0], round(float(r[1]), 1), round(float(r[2]), 1)] for r in src_rows[:60]],
                               "port": [[r[0], round(float(r[1]), 1), round(float(r[2]), 1)] for r in fm_rows[:120:2]]}
            paths[inp] = cmp
    r.evidence["paths"] = paths
    walk_fm = ctx.get("walk_fm_seq")
    if walk is not None and walk_fm is not None:
        want = walk * scale
        got = walk_fm
        r.evidence["walk"] = {"ssf2_per_frame": round(walk, 3), "scale": scale,
                              "fraymakers_per_frame": round(got, 3)}
        if want > 0 and abs(got - want) / want > 0.08:
            differ_walk = f"walk: SSF2 {walk:.2f} a frame scales to {want:.2f}, Fraymakers {got:.2f}"
        else:
            differ_walk = None
    else:
        differ_walk = None

    def first_move(runs):
        # the first run that is not the neutral pose, and the runs that follow it. a fall or a
        # landing of a few frames ahead of it is the fighter settling where it was put back, not
        # a move it made
        def neutral(k, a, n):
            # a stand or an idle, however the engine spells the one it drifted into while parked
            # (a bored idle a few seconds in is still the fighter waiting)
            a = a.lower()
            if a.startswith(("stand", "idle")):
                return True
            # a walk or a crouch ahead of the move is the way held before its attack (a tilt's
            # input), when something follows it (the source crouches under a held down; its
            # crouch attack is a move of its own)
            if (a == "crouch" or a.startswith(("crouch_in", "crouch_loop", "walk"))) and k + 1 < len(runs):
                return True
            # a fall or a landing AHEAD of the move is the fighter settling where it was put back,
            # not a move. a heavy fighter's landing runs longer (simon lands over ~6 frames of
            # land_light and fall_loop), and fall/land are state names a move never carries, so a
            # leading one is settling up to a length no real move is that short (moves run 15+).
            settling = a.startswith(("fall", "land")) and n <= 12
            return settling and all(x.lower().startswith(("stand", "idle", "fall", "land"))
                                    for x, _ in runs[:k])
        i = next((k for k, (a, n) in enumerate(runs) if not neutral(k, a, n)), None)
        return runs[i:] if i is not None else []

    def family(label, want):
        # the move, its air variant, and the numbered or staged pieces the engine splits it into
        l = label.lower()
        return l == want or l.startswith(want + "_") or (want.startswith(l.rstrip("0123456789")) and l[:1] == want[:1] and len(l) > 2)

    # a move whose own frame script loops back on itself is HELD: a charge, a reflector, a
    # steerable recovery -- it plays for exactly as long as the input is held, so its length on a
    # single dispatch is not the same fact on the two engines and is not length-compared here (it
    # is still checked for playing the right move). C5 already reads these as "length chosen by
    # own code"; C6 reads the same set the same way.
    held = {h.lower() for h in held_moves(ctx)}
    # a move that transforms the fighter into another has no Fraymakers counterpart (no character
    # swap), so the converter struck the call; the source still plays the transform, which is not a
    # difference the port can be held to. Skip it.
    transforms = {t.lower() for t in transform_moves(ctx)}
    same, differ, missing = [], [], []
    if differ_walk:
        differ.append(differ_walk)
    elif walk is not None and walk_fm is not None:
        same.append(f"walk: {walk:.2f} a source frame, {walk_fm:.2f} here")
    for inp in C6_ACTIVE:
        if inp in fm_undriveable:
            # the gate could not stage this input on the port (a recovery on a fighter that snaps
            # back to the floor when set above it): the move plays from a real jump, so this is a
            # staging limit, not a difference. report it, do not fail on it.
            same.append(f"{inp}: not staged on the port (recovery on a floor-snapping fighter; the gate can't launch it from the air)")
            continue
        a, b = first_move(ssf2.get(inp, [])), first_move(fm.get(inp, []))
        if not a and not b:
            continue
        if not a or not b:
            missing.append(f"{inp}: {'SSF2' if not a else 'Fraymakers'} did nothing "
                           f"(the other played {(b or a)[0][0]})")
            continue
        sa, fa = a[0]
        want = to_fm.get(sa, sa).lower()
        if want in transforms:
            same.append(f"{inp}: {want} transforms the fighter (no Fraymakers character swap); skipped")
            continue
        # the move on the other side is every consecutive run of its family: a grounded special
        # that goes airborne is one move in two animations
        fm_runs = []
        for lbl, n in b:
            if family(lbl, want) or (not fm_runs and family(lbl, want.split("_")[0])):
                fm_runs.append((lbl, n))
            else:
                break
        if not fm_runs:
            differ.append(f"{inp}: SSF2 {sa} -> expected {want}, Fraymakers played {b[0][0]}")
            continue
        fb = sum(n for _, n in fm_runs)
        # the source's move is its consecutive runs of the same family too: a grounded special
        # that goes airborne continues under its air label (zelda's up special: 35 frames of
        # b_up, then 4 of b_up_air, then helpless), and the port plays those 39 as one animation
        a_runs = [a[0]]
        for lbl, n in a[1:]:
            if family(to_fm.get(lbl, lbl).lower(), want):
                a_runs.append((lbl, n))
            else:
                break
        fa = sum(n for _, n in a_runs)
        # the source's run is the move's own length whenever the source ended it (into anything)
        # rather than the trace ending first; the port's is, unless a loop is part of it (a jump
        # is as long as the air) or the floor cut it (a landing follows)
        ends_a = len(a) > len(a_runs)
        after_b = b[len(fm_runs)][0].lower() if len(b) > len(fm_runs) else ""
        # the label after the source's run, mapped to this engine's spelling so a landing reads
        # the same on both (heavyland -> land_heavy)
        after_a = to_fm.get(a[len(a_runs)][0], a[len(a_runs)][0]).lower() if len(a) > len(a_runs) else ""
        loops_b = any("_loop" in lbl.lower() for lbl, _ in fm_runs)
        # the move was interrupted, not ended by its own code, so its length is not its own:
        #  - the floor cut a recovery dispatched from the ground (it rises and lands), or
        #  - the fighter went into hitstun mid-move. driven solo, the only thing that can hit it is
        #    its own hitbox: sandbag's up special is a bomb it holds and sets off, and the blast
        #    launches sandbag itself (b_up -> hurt -> flying), a self-hit fraymakers has no item to
        #    reproduce. either way verify the right move plays, do not length-compare the cut run.
        interrupted = lambda s: s.startswith("land") or s.startswith("hurt") or s in ("tumble", "flying", "falling")
        cut_b = interrupted(after_b)
        cut_a = interrupted(after_a)
        cut = cut_a or cut_b
        # the move ran its own length on both engines (source ended it itself, no loop stretches it,
        # the port is within a frame or two of twice the source) -- a real 1:1, whatever comes after
        matches = ends_a and not loops_b and abs(fb - 2 * fa) <= 2
        # held is keyed on the entity's own animation names (jab1), while `want` is the mapped family
        # base (jab); the labels the port actually played are the real names, so test those too
        if want in held or any(lbl.lower() in held for lbl, _ in fm_runs):
            same.append(f"{inp}: {want} held (length set by the input, played {fb} here; not length-compared)")
        elif matches:
            # a match is a match even if a landing follows: the move finished, then the fighter came
            # down. checking this BEFORE `cut` is what lets an airborne-measured recovery compare
            # 1:1 instead of being skipped for the landing that trails it.
            same.append(f"{inp}: {want} {fa}x2={fb}")
        elif cut:
            # it differs AND an interruption explains why: the floor cut a grounded recovery short,
            # or the fighter took a hit mid-move (driven solo, its own hitbox -- sandbag's self-bomb).
            same.append(f"{inp}: {want} interrupted ({'source' if cut_a else 'here'} hit the floor or "
                        f"took a hit mid-move); played {fa} source frames, {fb} here; not length-compared")
        elif ends_a and not loops_b:
            differ.append(f"{inp}: {want} runs {fa} source frames ({2 * fa} expected), Fraymakers {fb}"
                          + (f" then {after_b}" if after_b else ""))
        else:
            same.append(f"{inp}: {want} (identity)")
    r.evidence.update({"same": same, "differ": differ, "missing": missing,
                  "ssf2": {k: v[:5] for k, v in ssf2.items()}, "fraymakers": {k: v[:5] for k, v in fm.items()}})
    if missing:
        r.fail(f"{len(missing)} input(s) did something on one engine only: {missing[0]}")
    if differ:
        r.fail(f"{len(differ)} input(s) differ: {differ[0]}")
    if r.status == "PASS":
        r.note(f"{len(same)} inputs play the same move for the same frames on both engines")


CHAR_GATES = [("C1 convert", c_convert), ("C2 publish", c_publish), ("C3 boot", c_boot),
              ("C4 physics", c_physics), ("C5 moves", c_moves), ("C6 ssf2", c_ssf2)]


def verify_char(cid, opts):
    ctx = {"id": cid, "ssf": source_ssf(cid, "character"),
           "dir": os.path.join(ROOT, "characters", cid), "opts": opts}
    print(f"== character {cid}", flush=True)
    if not os.path.exists(ctx["ssf"]):
        print(f"  no source at {ctx['ssf']}")
        return None
    results = run_gates_with_ahead(("char", cid), CHAR_GATES, ctx)
    if "engine" in ctx:
        if ctx.get("engine"):
            ctx["engine"].close()
    if "ssf2" in ctx:
        ctx["ssf2"].close()
    return results


# ----------------------------------------------------------------------------------------------
# reports
# ----------------------------------------------------------------------------------------------
def save(kind, sid, results):
    os.makedirs(os.path.join(REPORTS, kind), exist_ok=True)
    doc = {"kind": kind, "id": sid, "when": time.strftime("%Y-%m-%d %H:%M"),
           "verdict": "PASS" if all(r.status != "FAIL" for r in results) else "FAIL",
           "gates": [r.d() for r in results]}
    with open(os.path.join(REPORTS, kind, f"{sid}.json"), "w", encoding="utf8") as f:
        json.dump(doc, f, indent=1)
    return doc


def summary(docs):
    lines = ["# verify summary", "", f"run {time.strftime('%Y-%m-%d %H:%M')}", ""]
    for kind in ("stage", "char"):
        rows = [d for d in docs if d["kind"] == kind]
        if not rows:
            continue
        gates = [g["gate"].split()[0] for g in rows[0]["gates"]]
        lines += [f"## {kind}s", "", "| subject | " + " | ".join(gates) + " | first failure |",
                  "|---|" + "---|" * (len(gates) + 1)]
        for d in rows:
            cells = [g["status"] for g in d["gates"]]
            first = next((f"{g['gate']}: {g['why'][0]}" for g in d["gates"] if g["status"] == "FAIL"), "")
            lines.append(f"| {d['id']} | " + " | ".join(cells) + f" | {first[:120]} |")
        lines.append("")
    passed = sum(1 for d in docs if d["verdict"] == "PASS")
    lines.append(f"**{passed} of {len(docs)} subjects pass every gate.**")
    os.makedirs(REPORTS, exist_ok=True)
    with open(os.path.join(REPORTS, "summary.md"), "w", encoding="utf8") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines[-(len(docs) + 8):]))


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 2
    # both engines run eight frames a tick by default (the project's pacing rule); the
    # frame-paced gates count the engine's own frames so they measure the same at any rate
    opts = {"ssf2": "--ssf2" in argv, "keep": "--keep" in argv, "speed": 8}
    if "--deep" in argv:
        C6_ACTIVE.extend(i for i in C6_INPUTS_MORE if i not in C6_ACTIVE)
        # a down special can leave the fighter changed for what comes after (zelda's turns her into
        # sheik on the source, and every move after it was sheik's), so it goes last
        if "down+special" in C6_ACTIVE:
            C6_ACTIVE.remove("down+special")
            C6_ACTIVE.append("down+special")
    for a in argv:
        if a.startswith("--speed="):
            opts["speed"] = max(1, min(8, int(a.split("=", 1)[1])))
    args = [a for a in argv if not a.startswith("--")]
    subjects = []
    if args[0] == "stage":
        subjects = [("stage", a) for a in args[1:]]
    elif args[0] == "char":
        subjects = [("char", a) for a in args[1:]]
    elif args[0] == "sample":
        subjects = [("stage", s) for s in SAMPLE_STAGES] + [("char", c) for c in SAMPLE_CHARS]
    elif args[0] == "corpus":
        if "--stages" in argv or "--chars" not in argv:
            subjects += [("stage", os.path.basename(p)[:-4])
                         for p in sorted(glob.glob(os.path.join(SSFS, "stages", "*.ssf")))]
        if "--chars" in argv or "--stages" not in argv:
            subjects += [("char", os.path.basename(p)[:-4])
                         for p in sorted(glob.glob(os.path.join(SSFS, "*.ssf")))
                         if not os.path.basename(p).startswith(("DAT", "misc"))]
    else:
        print(__doc__)
        return 2
    if not os.path.exists(BIN):
        print(f"no binary at {BIN}: cargo build --release first")
        return 2
    docs = []
    # the source's engine comes up while the first subject converts and its port boots; the
    # cross-engine gate finds it ready. nothing else runs on it until then
    if opts["ssf2"] and subjects:
        opts["ssf2_boot"] = threading.Thread(target=prewarm_ssf2, daemon=True)
        opts["ssf2_boot"].start()
    # the next subject's conversion and package build run while this one is being driven:
    # they are the converter's cpu, and driving is waiting
    ahead = {}
    def prepare(kind, sid):
        ahead[(kind, sid)] = threading.Thread(target=lambda: convert_ahead(kind, sid, opts), daemon=True)
        ahead[(kind, sid)].start()
    if len(subjects) > 1:
        prepare(*subjects[1])
    for i, (kind, sid) in enumerate(subjects):
        t = ahead.pop((kind, sid), None)
        if t is not None:
            t.join()
        if i + 2 < len(subjects):
            prepare(*subjects[i + 2])
        results = verify_stage(sid, opts) if kind == "stage" else verify_char(sid, opts)
        if results:
            docs.append(save(kind, sid, results))
    if "--shutdown" in argv:
        close_live()
    if docs:
        summary(docs)
    return 0 if all(d["verdict"] == "PASS" for d in docs) else 1


def prewarm_ssf2():
    """boot (or adopt) the SSF2 engine ahead of its first use, on the source's default stage."""
    try:
        s = Ssf2Engine("prewarm")
        s.boot(["--char", "mario,mario"])
    except Exception as e:  # a failed prewarm only means the gate boots it itself
        print(f"[prewarm] ssf2 boot ahead of time failed: {e!r}", file=sys.stderr)


# conversions done ahead of their subject's turn: (kind, id) -> [Result, ...]
AHEAD = {}


def convert_ahead(kind, sid, opts):
    """the convert and publish gates of a subject, run early; their results are handed to the
    subject's own run so the work is not done twice."""
    try:
        if kind == "char":
            cid = sid
            ctx = {"id": cid, "ssf": source_ssf(cid, "character"), "dir": os.path.join(ROOT, "characters", cid), "opts": opts}
            gates = CHAR_GATES[:2]
        else:
            cid = f"{sid}ssf2"
            ctx = {"id": sid, "cid": cid, "ssf": source_ssf(sid, "stage"), "dir": os.path.join(ROOT, "stages", cid), "opts": opts}
            gates = STAGE_GATES[:2]
        AHEAD[(kind, sid)] = (ctx, run_gates(gates, ctx, quiet=True))
    except Exception as e:
        print(f"[ahead] {kind} {sid}: {e!r}", file=sys.stderr)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
