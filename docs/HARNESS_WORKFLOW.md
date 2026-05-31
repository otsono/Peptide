# Harness Workflow — iterating on characters end-to-end

This is the cold-start doc for working the converter ↔ FrayTools ↔ Fraymakers
loop. It is practical and paths-heavy on purpose. Every command/path/behavior
below was verified against the code in this repo on 2026-05-30; anything not
re-verified is marked `TODO: verify`.

There are **two** harnesses, doing two different jobs:

| Harness | Dir | Drives | Transport | Answers |
|---|---|---|---|---|
| FrayTools (Tier 3) | `tools/fraytools-harness/` | the user's FrayTools editor | Chrome DevTools Protocol | "does it render/lay out right? publish me a `.fra`" |
| Fraymakers | `tools/peptide/` | the user's Fraymakers engine | patched bytecode + loopback TCP | "does it behave at runtime? load/spawn/move/read state" |

---

## 1. Repo orientation

### Branches (verified — corrects a common stale belief)

- **`main` is the live branch.** It holds the converter **and** the merged
  Fraymakers harness work (merge commit `23ddec13` "land headless custom-load + spawn").
- **`fraymakers-match-harness` is NOT ahead of main.** It is **21 commits behind**
  (`git rev-list --count fraymakers-match-harness..main` = 21; `main..fraymakers-match-harness` = 0).
  Its tip (`1160319c`) is an ancestor of main. Don't go there expecting newer harness code — main has it all plus more.
- `steam-shim-experiments` also exists (experimental; not the iteration path).
- Verify any time with: `git rev-list --count fraymakers-match-harness..main`.

### Where converter output goes

- CLI default output dir: `./characters/<id>/` (`src/main.rs:20`, `--output` defaults to `./characters`).
- `characters/` is **gitignored** (`.gitignore:13`) — 0 tracked files. Output is on-disk only.
- **Rename history that bites you** (old → current):
  - `library/scripts/Character/` → `library/scripts/<Pascal>/` (`src/haxe_gen.rs:26`)
  - `library/sounds/*.ogg` + `sounds_manifest.json` → `library/audio/*.wav` (`src/haxe_gen.rs:440`)
  - `library/entities/Character.entity` → `<Pascal>.entity`; `menu.entity` → `Menu.entity`
- The converter **does not wipe** a char's output dir before writing — old files linger. See §6 and `docs/cleanup_report.md`.

### Converter inputs

- SSF2 source files live **outside** this repo: `~/.openclaw/workspace-main/ssf2-ssfs/<id>.ssf` (46 inputs).
- Note: `deespear` has output history but **no input** (orphan, deleted in cleanup); `chibirobo`/`dedede` have inputs but were never converted.

### Where Fraymakers reads custom mods

- `~/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/<id>/`
  containing `<id>.fra` + `meta.json` (verified: only `mario/` and `sandbag/` installed, 2 files each).
- The publish step (§2) lands the `.fra` here. The engine boot dir is
  `~/Library/Application Support/Steam/steamapps/common/Fraymakers/` (override with `FRAY_DIR`).

### Regenerating a character (CLI)

```
cargo build --release
./target/release/ssf2_converter /Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/<id>.ssf
#   → writes ./characters/<id>/   (add -o <dir> to redirect, --name to override id)
```

- `rebuild-sandbag.sh` wraps this for sandbag specifically (build + convert + sprite count).
- ⚠️ `tools/fraytools-harness/README.md`'s loop uses `--ssf2 / --out` flags — those are **stale**. The current CLI takes the input as a **positional** arg + `-o/--output`. `TODO: verify` the README and fix.

---

## 2. The FrayTools harness — publish step

Code: `tools/fraytools-harness/export-in-fraytools.js` (275 lines). Setup: `cd tools/fraytools-harness && npm install` (pulls `chrome-remote-interface`; `node_modules/` is gitignored).

### What it does

Drives the user's **local** FrayTools (Electron + Chrome DevTools Protocol) to run FrayTools' own **Publish** (the Fraymakers Content Exporter) on a converted project — no clicking. Steps (all via FrayTools' own runtime objects, no pixel coords):

1. Attach to FrayTools on `--port` (default 9222), or launch it with `--remote-debugging-port`.
2. Walk the React 16 fiber tree to find the **controller** component (the one exposing `getLibraryDirectory` + `openProject` + `publish`); stash as `window.__ctrl`.
3. `__ctrl.openProject(path)` if the project isn't already open (re-stashes the controller afterward — a project load can remount it).
4. Close any stale Publish dialog, `showPublishDialog(false)`, then click **"Publish All"** (force `publish()` is the fallback). "Publish All" hits every configured output folder — i.e. both `<projectDir>/build/` and the Fraymakers `custom/<id>/` dir the converter wired up.
5. Poll `<projectDir>/build/` for a freshly-written `.fra` (waits for the file size to stop growing), print its path on **stdout**.

### The cold-launch race (and its fix)

On a cold launch, FrayTools' `/json/version` HTTP endpoint answers `200` **before** the renderer registers an inspectable page target — so `CDP({port})` would throw `"No inspectable targets"`. Fix: `waitForTarget(port, timeoutMs)` (`export-in-fraytools.js:89`) polls `/json/list` until a `page`/`webview` target with a `webSocketDebuggerUrl` exists, and only then connects. This was the cold-launch publish failure.

### Invoke for a single character

```
node tools/fraytools-harness/export-in-fraytools.js \
  --project /Users/jimmy/.openclaw/workspace-main/ssf2-fraymakers-converter/characters/<id>/<id>.fraytools \
  [--fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools"] \
  [--port 9222] [--settle 6000] [--timeout 120000]
# stdout = absolute path to the published .fra
```

This is exactly what the converter GUI's **"Export in FrayTools"** button shells out to (`ssf2-converter-gui/src/platform.rs:64`, `main.rs:351`).

For **visual/layout** validation (box geometry, rendered pivots) use `harness.js` instead — it opens an entity, navigates to an animation+frame via Redux `store.dispatch`, extracts box geometry, captures a PNG, and emits a JSON report. Pair with `cargo run --bin compare_boxes` for a numeric drift verdict.

### Known fragilities

- **Stale modal/route state** in a running FrayTools can make the controller walk or the publish click fail. The script closes a stale Publish dialog first, but if the controller is unreachable (`could not locate FrayTools controller`), **restart FrayTools** and re-run.
- It **leaves FrayTools running** on success (attach model) so the next call reuses it. If CDP attach starts failing mid-session, kill FrayTools and let the script cold-launch a fresh instance.
- Tested against **FrayTools 0.4.0**; role detection is by method/prop presence, not class name, so it's somewhat version-tolerant.

---

## 3. The Fraymakers harness — engine driver

Code: `tools/peptide/` — Rust. Two bins:
- **`peptide`** (`src/main.rs`, ~1800 lines) — parses the engine's HashLink bytecode (`hlbc` crate), injects a per-frame dispatch block into `fraymakers.Main.update`, writes a patched copy. Also has read-only inspection subcommands.
- **`peptide-bridge`** (`src/bin/peptide-bridge.rs`) — the loopback TCP **server/bridge** (the injected engine code is the TCP *client*).

### Boot model (`run.sh`)

```
./run.sh "<command>" [seconds]      # FRAY_DIR=... overrides the install path
./run.sh "s commandervideo thespire commandervideoassist" 20
```

What `run.sh` does each run (Steam sandbox wipes anything added to the install dir, so it **recreates everything every time** and never mutates the pristine engine):

1. `peptide hlboot-sdl.dat _conn.dat connect <port> <token>` — patch a **copy**. `hlboot-sdl.dat` is the patch **source**, never written.
2. Write `steam_appid.txt` (so a direct `./hl` launch doesn't bounce through Steam).
3. Start `peptide-bridge serve --port <port> --token <token>`, queue the command on its stdin.
4. `cd "$FRAY_DIR" && DYLD_LIBRARY_PATH=. ./hl _conn.dat` — launch the patched copy.
5. On exit, delete `_conn.dat` + `steam_appid.txt`, kill procs.

Port: `run.sh` randomizes to 18000–19999; `peptide-bridge`'s built-in default is `17999`. The patched engine waits for content load (title "press any button" state), dials the socket (auth handshake), then processes commands per-frame on the main/render thread.

### Don't stall on closing old Fraymakers engine instances

Orphaned `hl` processes from earlier harness boots occasionally wedge into
uninterruptible sleep (`ps` shows `STAT` containing `U`, e.g. `UNE`). In that
state they ignore `SIGKILL` entirely and only clear on a full Mac restart.
Crucially, **a wedged orphan does not block a new engine from launching** — it
just sits there consuming nothing. Waiting on it, though, blocks the harness
forever.

**Rule: the "kill prior instance" step gets a hard 2-second budget — then move on.**
If `kill -9` hasn't reaped the process within ~2s, treat it as wedged, ignore
it, and launch the new engine anyway. Do **not** loop, retry, escalate, or wait
it out — there is no command that clears a `U`-state process short of a reboot,
so any further effort is pure wasted time.

### Commands (verified in `src/main.rs` — dispatched on the first byte)

| Cmd | Meaning | Ack / readback |
|---|---|---|
| `p` | ping / liveness | `PONG` (`main.rs:572`) |
| `c` | console passthrough — `Tildebugger.console.runCommand("help")` | `RAN` (`main.rs:568`) |
| `s <char> <stage> <assist>` | **start match (self-bootstrapping)** — first runs the custom-load core itself (idempotent: skipped if `getPXFResource("private::sandbag")` is already non-null, e.g. a prior `l`), then `createMode` a real `TrainingMode` and `FraymakersMode.startMatch({characters, matchSettings, pauseMenu})`. **No prior `l` needed** — `s sandbag thespire commandervideoassist` works in one swoop. Runs the engine's own offline-match flow (gates transition, menu teardown). | `LAUNCHED <char> <stage> <assist>` |
| `q` | is a match live? (`_matches` length / `currentMatch`) | `Q:MATCH_LIVE` / `Q:NO_MATCH` / `Q:MATCHES_NONEMPTY` (`main.rs:685`) |
| `k` | dump pool keys + UGC-discovery diagnostics (reveals the real namespace of headless-loaded content) | `K:...`, `K:DIRS_QUEUED>0/=0` (`main.rs:689`) |
| `l` | **synchronous custom-`.fra` load** (headless, main thread) — see recipe below | `L:...`, `SPR:1`/`SPR:0` (`main.rs:738`) |
| `m` | **move-dispatch** — `Character.toState(CState.JAB)` on player-0 (internal state-machine dispatch, **not** key-press simulation) | `M:JAB` / `M:NOMATCH` (`main.rs:716`) |
| `t` | **telemetry/state readback** — `Character.getStateName()` on player-0 | `T:<state>` / `T:NOMATCH` (`main.rs:735`) |

Short-name resolution (`s`/`l`): a bare `sandbag` (no `::`) is tried against `private::`, `custom::`, `public::`, `global::` prefixes in order; first existing resource wins (`main.rs:637`). `private::` is first so a bare name resolves to headless-loaded custom content (where `l` and `s`'s self-bootstrap register it). Or pass a full `namespace::package.id`.

**Multi-command sessions:** `run.sh` sends one command per boot, but `m`/`t`/`q` probes need the *same* live match (a reboot loses it). Use `runseq.sh <boot_wait_s> <gap_s> "cmd1" "cmd2" …` to feed a gapped sequence into one engine session. `boot_wait_s` ~32 (engine boot→READY ≈ 30s); `gap_s` 6–9. Env: `FRAY_ENGINE_LOG=<file>` captures engine stdout, `FRAY_TAIL` extra hold after the last command. Example: `./runseq.sh 32 6 "s sandbag thespire commandervideoassist" "p" "q" "t" "m" "t"`.

`peptide` read-only inspection subcommands (for re-deriving findices — **always re-verify**): `dis <findex>`, `typefields <type>`, `fnsof <type>`, `fninfo <findex>`, `callers <findex>`, `strgrep <s>`, `whoref <s>`, `inspect`.

### Minimal-boot recipe — loading a custom character headless (the `l` command)

The async UGC/worker path stalls headless, so the harness builds the resource synchronously on the main thread (`main.rs:738`–`798`):

1. Construct a `pxf.io.Resource` for the `.fra`, force `_isAbsolute=true` + `_type=PXF`, point `_filePath` at the custom `.fra` (currently **hardcoded** to `…/custom/sandbag/sandbag.fra`, `main.rs:761` — `TODO: verify`/parameterize when generalizing past sandbag).
2. Set `ResourceManager.requiredMediaIds = ["*"]` **before** `fetchThreaded`, so the engine's own media-preload closure (run by `finishLoading`) preloads all entities into `_data.entityMap` + the namespaced sprite cache (`main.rs:772`).
3. `Resource.fetchThreaded` (synchronous internally: `File.getBytes` → `PXFResource.createFromBytes` → `set_DataAsPxf`) → `finishLoading` → `ResourceManager.addResource` (pushes onto the ordered `pool`).
4. Deterministically build the sprite entity: `PXFResource.cacheSpriteEntityData(pxf, idx)` (no flaky preload, no `UnsafeCast`), then re-cache the entity under the **bare** key and under the **buried-VFX key** `private::sandbag.sandbag` — the value `Character.hx:762` reads as `statsProps.spriteContent` (`getContent("sandbag")` → `getFullyQualifiedResourceId` + `.sandbag`). Sourcing the entity from `getPXFSpriteEntity` (not the null `entityMap.get`) is what made `SPR:1` non-null (the resolved "buried-VFX null"). (`main.rs:777`–`796`)
5. To **reveal** the rendered match non-destructively, hide `CoreEngine.menuContainer.displayObject.visible` (the match renders in the sibling `gameContainer`). (`main.rs:799`)

Full RE chain + verification proofs: **`docs/ENGINE_RE_MAP_v2.md`** (findices re-derived by name; supersedes older docs) and **`docs/HARNESS_MT_VERIFIED.md`** (move-dispatch + telemetry confirmed in a live `commandervideo` match, clean, no `error.log`).

---

## 4. The end-to-end iteration loop

```
# 1. Make a converter change
$EDITOR src/<...>.rs

# 2. Rebuild + regenerate the target character
cargo build --release
./target/release/ssf2_converter ~/.openclaw/workspace-main/ssf2-ssfs/<id>.ssf
#    → ./characters/<id>/   (⚠ verify it's FRESH — see §6 stale-output trap)

# 3. Publish via the FrayTools harness → .fra lands in Fraymakers' custom/<id>/
node tools/fraytools-harness/export-in-fraytools.js \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 4. Boot the Fraymakers harness, dispatch, observe
cd tools/peptide
./run.sh "s <id> thespire <assist>" 20     # spawn into a real match
#   or, for the headless synchronous load path:  ./run.sh "l" 20
#   then probe:  ./run.sh "t" 8   (read state)   ./run.sh "m" 8  (drive a move)

# 5. Compare behavior vs expected → fix in the converter → repeat
```

**Which harness to use when:**
- **FrayTools-side** (`harness.js` / `export-in-fraytools.js`): visual/layout ground truth — box geometry, pivots, rendering, and producing the publishable `.fra`. Use `compare_boxes` for a numeric verdict.
- **Fraymakers-side** (`run.sh`): runtime behavior — does the char load, spawn, animate, transition states, respond to a move dispatch. This is where freeze/crash/physics bugs surface.

---

## 5. Physics + state probing

What the harness exposes **today** (player-0 of the live match):
- `q` — is a match live at all.
- `t` — current state **name** via `Character.getStateName()` (`main.rs:722`).
- `m` — drive a state transition via `Character.toState(CState.JAB)` (`main.rs:721`) — internal dispatch, deterministic, no input simulation.

**Extending it (the bytecode-authoring pattern).** The injector in `peptide` (`src/main.rs`, the `connect`/`connect_edit` path) is the template for adding probes:
1. Resolve engine symbols **by name**, never by hardcoded findex: `require_fn(code, "<method>", Some("<Type>"))`, `find_field(code, ty, "<field>")`, `find_type` / `require_type`. (Hardcoded findices like the `resource_ctor`/global-index constants are build-specific and must be re-verified — `dis`/`fninfo`/`strgrep` are for exactly that.)
2. Allocate constants: `add_string_const` (real String objects), `add_int`, `add_string`.
3. Emit opcodes into `Main.update` and wire the jump table (see the `m`/`t`/`l` handler jump comments at `main.rs:1583`–`1626`). New command = new first-byte branch with its own ack string.

To read **position/velocity** (not yet exposed), add a `t`-style readback: resolve the relevant `Character` getter/field by name and emit a socket-write of the value. Same pattern as `getStateName`.

**The harder ceiling (memory'd direction).** Match *launch* and richer match/menu/mode interaction bottom out in the engine's **hscript mode system** (`TrainingMode`/`VsMode` are interpreted hscript, invoked by the engine's dynamic mode/menu dispatch). Reconstructing that purely in hand-emitted bytecode is not tractable; the durable design is the **bytecode bridge (transport + dispatcher + strings) for control/observe, with launch/rich-instrumentation living in content-side hscript** (a generated `CustomApiObject` launcher authored through the FrayTools script pipeline — Fraymakers `.fra` hscript is compiled `HS_*` entries, not hand-written bytecode). See `memory/project_fraymakers-engine-internals.md`. `TODO: verify` — if a future "pre-compiled `.hl` for richer instrumentation" path is pursued, document it here when proven.

---

## 6. Pitfalls + workarounds

- **Stale-output trap — verify you're debugging FRESH output.** The converter merges into whatever's already in `characters/<id>/`; old renamed-away files (e.g. `scripts/Character/Script.hx` with a broken loop) silently persist. We deleted **7,297** such files corpus-wide — see `docs/cleanup_report.md`. Before trusting a runtime result, confirm the `.fra` you published came from a fresh regen (check `characters/<id>/library/scripts/<Pascal>/` mtimes; the dir should contain *only* current-converter paths).
- **Tool-channel corruption.** If bash output looks wrong/truncated, don't trust it — shasum-verify suspicious output and drop a canary (`echo "canary $(date +%N)"`) to confirm the channel is intact before acting on it.
- **Approval gates / wedges.** Avoid commands that surface approval prompts (`sudo`, network, risky-looking deletes). Prefer file tools (Read/Edit/Write) over `cat`/`sed` for inspection.
- **Wedge prevention:**
  - Bash stdout is capped ~100 KB — pipe big catalogs through `wc -l` or `| head -c 100000`; write bytecode/disassembly dumps to temp files and read them in chunks.
  - Long status belongs in a **committed doc**, not inline chat.
  - **Commit + push frequently** so progress lives in git regardless of tool-channel state. `characters/` is gitignored, so converter output is never the deliverable — docs and `src/` changes are.

---

## 7. Where to find authoritative info

| Doc | What it holds |
|---|---|
| `docs/ENGINE_RE_MAP_v2.md` | Engine RE map, findices **re-derived by name** (2026-05-30 build). Supersedes findex citations in older docs. Always re-verify a findex with `fninfo` before use. |
| `docs/HARNESS_MT_VERIFIED.md` | Move-dispatch (`m`) + telemetry (`t`) verified in a live match, clean. |
| `docs/HARNESS_AUDIT.md` | What the engine-side harness has vs. needs (load → drive → measure → assert → scale to 47 chars). |
| `docs/cleanup_report.md` | The stale-output purge (7,297 files); the rename history that causes the stale trap. |
| `docs/autonomous-sandbag-plan.md` | Older but useful: the sandbag end-to-end plan + stop condition (build out both harnesses to validate a port). |
| `handoff.md` (repo root) | Latest session handoff state. |
| `tools/peptide/README.md` | Boot model, IP boundary, command quick-start. |
| `tools/fraytools-harness/README.md` | Publish + box-geometry harness usage (⚠ its converter CLI flags are stale — see §1). |
| `memory/project_fraymakers-engine-internals.md` | The full RE narrative: injection vectors, the timing breakthrough, the hscript-launcher conclusion. |

**Branch ownership (verified):** `main` has the converter **plus** the fully merged harness work. `fraymakers-match-harness` is an older ancestor (21 commits behind main) — historical, not the place for new work. `steam-shim-experiments` is experimental.
