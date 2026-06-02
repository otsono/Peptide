# Validating a converted character

How to take a converted SSF2 character end-to-end and prove it is correct —
both in the **FrayTools editor** (does it lay out / render right?) and in the
**Fraymakers engine** (does it load, spawn, animate, and behave at runtime?).

There are **two** harnesses, doing two different jobs. Each lives under `tools/`
and contains **no Fraymakers/FrayTools code, assets, or strings** — they only
drive the user's *local* installs and speak standard protocols. Everything they
touch (the engine bytecode, patched copies, `.fra` packages, `node_modules/`)
stays on the user's machine and is git-ignored.

> **Copyright boundary — never publish.** The engine RE map below names methods,
> fields, and types as **facts** for interoperability, described in our own words.
> Never commit or publish the Fraymakers engine bytecode/Haxe/disassembly,
> FrayTools bundle code, SSF2 AS3, decompiled output, or any of their assets —
> they are copyrighted. See [`NOTICE.md`](NOTICE.md) "Reverse-engineering &
> copyright boundary".

| Harness | Dir | Drives | Transport | Answers |
|---|---|---|---|---|
| **FrayTools** (`peptide`) | `src/fraytools.rs` | the user's FrayTools editor | Chrome DevTools Protocol | "does it render / lay out right? publish me a `.fra`" |
| **Fraymakers** (`peptide`) | `src/` + `tools/` | the user's Fraymakers engine | patched bytecode + loopback TCP | "does it behave at runtime? load / spawn / move / read state" |

Both surfaces are subcommands of the one `peptide` binary; the detailed command surface lives in:
- [`docs/PEPTIDE_GUIDE.md`](docs/PEPTIDE_GUIDE.md) — `peptide export` (publish), `peptide harness` (box geometry + capture), `peptide render`, the bytecode patcher, the loopback bridge, and `tools/run.sh`. `compare_boxes` is a `dev-tools`-gated diagnostic bin.

This doc is the **cross-tool workflow + the engine RE map + the current
validation status**. (Long-term RE narrative also lives in
`memory/project_fraymakers-engine-internals.md` and
`memory/project_fraymakers-match-launch.md`.)

---

## 1. The end-to-end iteration loop

```
# 1. Make a converter change
$EDITOR src/<...>.rs

# 2. Rebuild + regenerate the target character
cargo build --release
./build/release/peptide convert ../ssf2-ssfs/<id>.ssf
#    → ./characters/<id>/   (⚠ verify it is FRESH — see §6 stale-output trap)

# 3. Publish to .fra → lands in Fraymakers' custom/<id>/
./build/release/peptide export \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 4. Boot ONE persistent engine session, then drive + observe it iteratively
./build/release/peptide session --full &        # boots + holds a live engine
#   wait for "engine READY" in:  ./build/release/peptide log
./build/release/peptide tell "spawn <id>"                       # start a real match
./build/release/peptide tell "match.getCharacters()[0].getStateName()"   # read state
./build/release/peptide tell "match.getCharacters()[0].toState(CState.JAB)"  # drive a move
./build/release/peptide log -n 20                # read everything the engine streamed back
./build/release/peptide tell "exit"             # clean shutdown when done

# 5. Compare behaviour vs expected → fix in the converter (never hand-edit output) → repeat
```

**The session IS the loop.** `peptide session` is the canonical way to test,
iterate on, and analyze a converted character (and the conversion scripts): it
boots the engine ONCE and HOLDS the TCP link open, so you send a command
(`peptide tell …`), read the engine's reply (`peptide log`), decide the next
command, and repeat — all against the *same* live match. That persistence is
what lets the game keep streaming TCP messages back while you run evals. Full
command surface in §3. (The old one-shot `tools/run.sh "<cmd>" <secs>` / fixed
`tools/runseq.sh` paths still exist for scripted runs, but reboot the engine per
invocation — use a session for interactive iteration.)

**Which harness when:**
- **FrayTools-side** (`peptide harness` / `peptide export`) — visual/layout
  ground truth (box geometry, pivots, rendering) and producing the publishable
  `.fra`. Pair with `compare_boxes` for a numeric verdict.
- **Fraymakers-side** (`peptide session`) — runtime behaviour (loads, spawns,
  animates, transitions state, responds to a move dispatch). This is where
  freeze / crash / physics bugs surface.

**Branch note:** `main` holds the converter **and** the merged harness work.
(Older `fraymakers-match-harness` / `steam-shim-experiments` branches are
historical — not the place for new work.)

---

## 2. FrayTools harness — publish + box geometry

Code: `src/fraytools.rs` (pure Rust; speaks CDP over HTTP + WebSocket directly,
no Node). It drives the user's local FrayTools (Electron + CDP) entirely through
FrayTools' own runtime objects — no pixel coordinates.

- **`peptide export`** runs FrayTools' own **Publish** (the Fraymakers
  Content Exporter) on a converted project and prints the published `.fra` path.
  "Publish All" writes both `<projectDir>/build/` and the Fraymakers
  `custom/<id>/` dir the converter wired up. This is exactly what the GUI's
  "Export in FrayTools" button shells out to.
- **`peptide harness`** opens an entity, navigates to an animation + frame via Redux
  `store.dispatch`, reads box geometry from the entity JSON, captures the stage
  as a PNG, and emits a JSON report whose `rendered_anchor` field is *FrayTools'
  own* placement of each box's pivot.

**Cold-launch race (fixed).** On a cold launch FrayTools' `/json/version` answers
200 before the renderer registers an inspectable page target, so a bare `CDP()`
threw "No inspectable targets". Both scripts now `waitForTarget()` — poll
`/json/list` for a real `page`/`webview` target before connecting.

Tested against **FrayTools 0.4.0**; role detection is by method/prop presence
(not class name), so it's somewhat version-tolerant. The script leaves FrayTools
running on success (attach model); if CDP attach starts failing mid-session,
kill FrayTools and let the script cold-launch a fresh instance.

### `compare_boxes` — numeric verdict

```
cargo run -p ssf2_converter --features dev-tools --bin compare_boxes -- \
  --ssf2 ../ssf2-ssfs/<id>.ssf \
  --char <id> --json /tmp/box.json [--tolerance 2.0]
```

Reads the harness JSON, parses the SSF2 source, matches boxes by type + size, and
reports per-box drift (FrayTools rendered anchor vs SSF2 intended position). It is
**split-aware** — split sub-animations (`strong_forward_attack`, `*_land`,
`grab_hold`, …) are mapped back to their source SSF2 anim + start-frame offset so
they verify instead of being skipped. Exits 0 on full pass, 1 on any failure.

---

## 3. Fraymakers engine harness (`peptide`)

Code: `src/` (Rust), with shell orchestration in `tools/`. Two bins:
- **`peptide`** parses the engine's HashLink bytecode (`hlbc` crate), injects a
  per-frame dispatch block into `fraymakers.Main.update`, and writes a patched
  copy. Also has read-only inspection subcommands.
- **`peptide`** is the loopback TCP server (the injected engine code is
  the client).

### Boot model — `peptide session` (canonical) + `tell` / `log`

`peptide session` is the primary surface for interactive, agent-driven testing:
one command boots a throwaway-patched engine, holds the loopback TCP link open,
and runs a command loop you feed over time.

```
peptide session [--full | --char <id>] [--dir D]   # boot + hold a live engine (long-lived)
peptide tell "<command>"                           # queue a command for the running session
peptide log  [-n N] [--follow]                     # print/tail the engine replies it mirrored
```

- **`session`** patches a *copy* of `hlboot-sdl.dat` → `_conn.dat` (the pristine
  boot file is never written), writes `steam_appid.txt`, launches `./hl _conn.dat`,
  binds the loopback server, and waits for the engine to dial in + reach READY.
  On clean exit (or `tell exit`) it kills the engine and removes the transient
  files. Needs `dangerouslyDisableSandbox` (the engine's SDL/Metal window).
  - `--full` = Title/UGC bridge boot, then drive it with `tell "spawn <id>"`
    once READY (the reliable path; avoids the headless filtered-load crash).
  - `--char <id>` = headless fast-boot baking that character.
  - `--no-boot --port N --token T` = attach to an engine launched elsewhere.
- **`tell`** appends one command to the session's control file; the daemon picks
  it up within ~50ms, translates it, and sends it. Commands are the same friendly
  vocabulary as everywhere else (`spawn`, `exit`, or any hscript expression).
- **`log`** prints the mirrored engine output (the canonical record of replies);
  `--follow` tails it live. Session state lives under `~/.peptide/session/`
  (override with `--dir` or `PEPTIDE_SESSION_DIR` for parallel sessions).

Why a session and not one-shots: it boots the engine ONCE and keeps it alive, so
you can send → observe → decide → send again against the SAME match, and the game
keeps streaming TCP back the whole time. The older one-shot `tools/run.sh
"<cmd>" <secs>` and fixed-sequence `tools/runseq.sh` reboot per run — fine for
scripted batches, wrong for iteration.

**Wedge discipline:** a crashed engine can leave an uninterruptible (UE) `hl
_conn.dat` orphan that `kill -9` can't reap (GPU/Metal wedge). Batch your checks
into ONE session; if orphans pile up, the box needs a reboot before more boots
behave. See `memory/project_peptide-env-wedge-lesson.md`.

### Commands

Humans type **full-word commands** (`peptide help` lists them); the bridge
translates them to the single-byte wire protocol the engine dispatches on. The
wire byte still works as an alias, so older scripts keep running. Friendly
vocabulary is one shared table: `src/interpreter.rs`.

> **The model is hscript-first.** The client command vocabulary (`COMMANDS` in
> `src/interpreter.rs`) is deliberately tiny — `help`, `spawn`, `eval`, `load`,
> `console`, `exit`, `hold`, `release`, `seq` — and **anything it doesn't
> recognize is forwarded to the `e` (eval) handler as hscript**, run through the
> engine's own interpreter. So instead of `move`/`state`/`physics`/`anim` sugar
> you write the call explicitly against the live match, e.g.
> `match.getCharacters()[0].getStateName()`,
> `match.getCharacters()[0].toState(CState.JAB)`,
> `match.getCharacters()[0].body.x`. The full Fraymakers script API (CState,
> HitboxStats, MatchModifier, …) is in scope; the in-engine helpers live in
> `commands.hsx`.

Live wire bytes (what the client actually sends):

| Command (aliases) | Wire | Meaning | Ack / readback |
|---|---|---|---|
| `spawn <char> [stage] [assist]` (start, launch, s) | `s …` | **start match (self-bootstrapping)** — idempotent custom-load core then `TrainingMode` + `FraymakersMode.startMatch`. | `LAUNCHED <char> <stage> <assist>` |
| `eval <hscript>` (e) | `e …` | parse + run the hscript text in the engine interpreter; **default for any unrecognized line** | `E:<result>` / `E:<error>` |
| `hold`/`release`/`seq` (i) | `i <mask>` | held-control bitmask fed to the engine input→action mapping; `seq` plays one mask per frame | (input applied) |
| `load` (l) | `l` | **synchronous custom-`.fra` load** (headless, main thread) — see §3.1 | `L:…`, `SPR:1`/`SPR:0` |
| `console` (c) | `c` | console passthrough (`Tildebugger.console.runCommand`) | `RAN` |
| `exit` (quit, stop, x) | `x` | clean engine shutdown (`hxd.System.exit`) | — |

**Legacy bytecode handlers** still spliced into the dispatch but **no longer
exposed as client commands** (slated for removal as their logic ports to
`commands.hsx`): `p` (ping/`PONG`), `m` (move dispatch via `commands::MOVES`
selector → `Character.toState`), `t` (state name), `q` (match-live), `v`
(physics), `a` (anim frame), `f`/`g` (frame-step/resume), `k` (pool keys). These
fire only if you send the raw byte; the friendly names that used to map to them
are gone, so prefer the hscript form. Per-state-change animation telemetry still
streams as `ANIM:<state>` (the bridge dedups to changes only) — the
animation-capture readback (criterion #5).

Short-name resolution (`spawn` / `load`): a bare `sandbag` (no `::`) is tried against
`private::`, `custom::`, `public::`, `global::` in order; first existing resource
wins. `private::` is first so a bare name resolves to headless-loaded custom
content. Or pass a full `namespace::package.id`.

**Multi-command sessions:** hscript probes need the *same* live match (a reboot
loses it). Use `tools/runseq.sh <gap_s> "cmd1" "cmd2" …` to feed a gapped sequence
into one engine session — it boots once (boot→READY budget via `FRAY_READY_BUDGET`),
fires the first command at READY, and paces the rest by `gap_s` (fractional OK).
Example:
`./tools/runseq.sh 6 "spawn sandbag" "match.getCharacters()[0].getStateName()" "match.getCharacters()[0].toState(CState.JAB)"`.

`peptide` read-only inspection (for re-deriving findices — **always re-verify**):
`dis <findex>`, `typefields <type>`, `fnsof <type>`, `fninfo <findex>`,
`callers <findex>`, `strgrep <s>`, `whoref <s>`, `inspect`.

### 3.0 Surviving Fraymakers updates (the version-compatibility gate)

Every Fraymakers update is a full HashLink **recompile** that renumbers function
indices, field slots, and type indices. Peptide is built so a new build is a
fast, self-diagnosing turnaround rather than an archaeology dig. The rules (full
write-up + version-bump checklist in
[`docs/PEPTIDE_DESIGN.md`](docs/PEPTIDE_DESIGN.md) "Version resilience"):

1. **Resolve by name, not index** — `find_fn`/`require_fn`, `find_type`, `find_field`, `find_native`. Names survive a recompile; pinned integers silently point at the wrong function.
2. **Fail loudly, never fall back to a pinned index** — a missing name aborts the patch instead of corrupting it.
3. **Prefer hscript over hand-emitted bytecode** — logic in [`commands.hsx`](commands.hsx) runs through the engine's own interpreter and is immune to index drift.
4. **Avoid mid-function opcode injection** — use `insert_ops_front`/`insert_ops_end` or hscript.

Every engine symbol the patcher needs is declared once in
[`src/manifest.rs`](src/manifest.rs). **`doctor`** is
the preflight that reads it:

```bash
# read-only: resolve every depended-on engine symbol against a bytecode file
peptide "<install>/hlboot-sdl.dat" _ doctor
```

It prints a grouped checklist (`[ ok ] name #findex` / `[MISS] name MISSING
(CRITICAL) — why`) and a summary (`71/71 resolved · 0 critical missing`). Run it
first against any new Fraymakers build to see exactly what (if anything) moved.
The live `connect` patch runs the **same** check at its top — rendering a progress
bar while resolving — and **aborts before mutating any opcode** if a critical
symbol is missing, so an incompatible build fails loudly instead of producing a
broken `_conn.dat`. In the GUI the bar shows in the boot modal ("Verifying engine
N/71"); in CLI/TUI it draws on stderr. **Version-bump loop:** run `doctor` → for
each `[MISS]`, find the new name with the inspection modes above and update both
`manifest.rs` and the `require_*` call in `connect_edit` → repeat until clean →
then the in-engine spawn-test below.

### 3.1 How custom content loads headless (the resolved blocker)

For a long stretch, custom/UGC content would not load in the direct `./hl`
launch: the engine prints `[API loaded no]` (Steam API not live in a direct
launch), and the normal UGC pipeline is **async** — `Resource.fetch` only pushes
a task onto `ThreadTaskManager.tasks`, and the worker thread that drains that
deque is never started in the harness boot, so the task sits forever and
`getPXFResource` returns null → `spawnPlayer` crashes on a null
`characterPxfContentMap`. Builtins (`assets/data/dat*.fra`) load synchronously
via `importManifest` and were unaffected.

**Resolution:** the `l` command (and `s`'s self-bootstrap) loads the `.fra`
**synchronously on the main thread**, bypassing the dead worker:

1. Construct a `pxf.io.Resource` for the `.fra` (`_isAbsolute=true`, `_type=PXF`,
   `_filePath` at the custom `.fra`), set `ResourceManager.requiredMediaIds=["*"]`
   so the engine's own media-preload runs, then `fetchThreaded` (which is
   internally synchronous: `File.getBytes` → `createFromBytes` → `set_DataAsPxf`)
   → `finishLoading` → `addResource` (pools it).
2. Deterministically build sprite entities with `cacheSpriteEntityData(pxf, idx)`
   over `PXFResource.entities`, then re-cache the entity under the **bare** key and
   the **buried-VFX key** `private::sandbag.sandbag` (the value `Character.hx`
   reads as `statsProps.spriteContent`). Sourcing the entity from the built
   `entityMap` (not a null `entityMap.get`) is what fixed the `Character.hx:769`
   buried-VFX null crash and gave reliable spawns (8/8 to `T:STAND`).

> **Still sandbag-specific.** The load currently builds its path/key from a
> hardcoded `custom/sandbag/sandbag.fra`. Generalizing it to build the path/key
> from the `s` argument is the remaining step for `s <anychar>` across the
> 47-character corpus.

The reversible-`hlboot`-swap + Steam-launch alternative (which would give a live
Steam API and load workshop content too, but writes a Steam file) was explored
and is **not** the chosen path — the synchronous in-process load stays within the
"don't modify the Steam install beyond `custom/<id>/`" constraint.

---

## 4. Engine RE map (move dispatch + telemetry + load)

Handles for extending the harness. **Findices drift between builds — resolve every
symbol by NAME** (`require_fn(code, "<method>", Some("<Type>"))`,
`find_field`, `find_type`); use `fninfo`/`dis`/`strgrep` to re-verify. The
indices below are a starting point, not a contract.

**Live Character path:** `MatchController` statics → `currentMatch` → `Match`
(type 634) → `.characters` (field 35, ArrayObj; `characters[0]` = player 0) →
`Character` (type 783). `_matches` is field 13; `elapsedFrames` is Match field 75
(a natural freeze oracle — advances iff `update()` ticks).

**Move dispatch:** `toState(char, cstateInt, animName)` is primary (`setState`
the 2-arg form). The move id is a **CState Int** read at runtime exactly as the
engine does: `GetGlobal($CState statics) → Field(N)`. Field indices: STAND=7,
FALL=24, JAB=68, DASH_ATTACK=69, TILT_FORWARD=70, TILT_UP=71, TILT_DOWN=72,
STRONG_FORWARD=75, STRONG_UP=78, STRONG_DOWN=81, AERIAL_NEUTRAL=82,
AERIAL_FORWARD=83, AERIAL_BACK=84, AERIAL_UP=85, AERIAL_DOWN=86,
SPECIAL_NEUTRAL=87, SPECIAL_SIDE=88, SPECIAL_UP=89, SPECIAL_DOWN=90, GRAB=96.

**Telemetry fields on `Character`:**
- state: `m_state` (Int CState id; name via `getStateName`), `stateName` (String).
- position: `body` → `x` / `y` (Float).
- velocity: `physics` → `x_velocity` / `y_velocity` / `currentVelocityX` / `currentVelocityY`.
- damage/%: `damage` → `_damage` / `_effectiveDamage` / `_maxDamage`.
- animation: `animation` → `currentAnimation` (String) / `currentFrame` / `totalFrames`.
- facing: `transformation.scaleX` (`isFacingLeft` = scaleX ≤ 0).

**Load path primitives (the synchronous publish chain):** `addResource`
(→ `poolHash`, what `getPXFResource` reads), `finishLoading` (→ per-type content
caches), `set_DataAsPxf`, `cacheSpriteEntityData`, `getPXFSpriteEntity`. A
constructed `PXFResource` always has a non-null `characterPxfContentMap` (the ctor
sets it unconditionally) — so a null at `spawnPlayer` always means the resource
was never fully constructed (never loaded), never a namespace bug.

> **The drive-and-read walk is verified live** against a builtin (`commandervideo`)
> match: `LAUNCHED → STAND → JAB (transient) → STAND → match-live`. (This was
> originally exercised via the legacy `m`/`t`/`q` wire bytes; the same walk is now
> driven with hscript — `toState(CState.JAB)`, `getStateName()`.) Sample state at
> frame cadence (~0.12s) — a single delayed sample misses short moves (jab returns
> to STAND in ~0.36s).

---

## 5. In-engine validation status

Stop-condition for "a converted character is validated" (the original
6-criterion mandate, sandbag as the reference character):

| # | Criterion | sandbag | mario |
|---|---|---|---|
| 1 | **Conversion clean** — exit 0; `conversion_log.json` triaged | **MET** | **MET** |
| 2 | **FrayTools layout match** — `compare_boxes` in tolerance | **MET** (§6) | not re-run |
| 3 | **Engine boots + spawns**, no crash | **MET** — spawn → `T:STAND` | **MET** — spawn → INTRO → STAND, stable |
| 4 | **Every move runs** via the internal control path | **MET** (spot-checked across categories) | **MET** — full moveset driven |
| 5 | **Animations play** (per-state `ANIM:` stream) | **MET** | **MET** |
| 6 | **Physics within tolerance** of `CharacterStats.hx` | readback live | readback live (dash_attack moves x −130→49) |

**Both reference characters drive their movesets in-engine (verified live).**
With `move <name>` (engine `toState`) every state produces the expected
`ANIM:<STATE>` transition + `M:OK` and recovers to `STAND`, no crash. mario's
full set was swept (jab, dash_attack, tilt f/u/d, strong f/u/d as full
`_IN`→`_ATTACK` chains, special n/s/u/d, aerial n/f/b/u/d, grab); sandbag was
spot-checked across the same categories. See "stale-`.fra` trap" below — mario's
earlier post-INTRO crash was a 3-day-old published `.fra`, not a live converter
bug; a fresh export boots and drives clean.

**Corpus convert status: 45/45 characters convert (`misc.ssf` = shared data, not
a character).** The two former failures (`chibirobo`, `dedede`) were a decompiler
OOM: a mis-parsed CFG range made `decode_range` read a garbage near-`u32::MAX`
argc/count for call/construct/newarray/newobject opcodes and allocate a ~240 GB
`Vec<Expr>` → SIGKILL/137. **Fixed** by clamping every `(0..argc/count).pop()`
loop to the operand-stack depth (`src/decompiler.rs`). Localized with the gated
memory-limit allocator now in `src/main.rs` (`CONV_MEM_LIMIT_MB`, default-off):
it makes `alloc` return null past a cap so Rust's alloc-error handler aborts with
the offending size + backtrace — reusable for any future OOM.

**Verified functional in-engine this session (spawn → STAND, drive moves,
animate, no crash) — 11 characters:** `sandbag`, `mario`, `kirby`, `bowser`,
`fox`, `marth`, `falco`, `captainfalcon`, `donkeykong`, `bomberman`, `blackmage`.
Each was regenerated from the current converter, re-exported via FrayTools, and
driven through `spawn` + `move <name>` (+ `physics` for the first five). mario was
the deep sweep (full 18-move set); the rest were spot-checked across jab /
special_neutral / grab categories — every one launched, reached `STAND`, and
returned `M:OK` for each dispatched move with no engine crash. This validates the
generic load+drive pipeline broadly, not just the two reference characters.

The last six were run unattended by `(/tmp)/batch_chars.sh`: per character it
regenerates, exports via FrayTools, then `tools/runseq.sh`-drives `spawn/state/move
jab/move special_neutral/physics` and records PASS only if it LAUNCHED + hit
`ANIM:STAND` + got `M:OK` + had no `rosetta error` in the engine log. Reusable
template for sweeping the rest of the 44 converting characters.

**The stale-`.fra` trap (re-export before trusting any runtime result).**
`characters/` and the published `.fra` are git-ignored, so an old `.fra` in
`custom/<id>/` silently survives converter changes. Mario was crashing the engine
right after its INTRO purely because `custom/mario/mario.fra` predated recent
fixes. Regenerate (`peptide convert`) **and** re-publish
(`peptide export`) before drawing any conclusion from a spawn test.

**Converter freeze fix — DONE and confirmed.** The user's central concern was
sandbag freezing the engine shortly after match start. Root cause was a converter
bug: the converter's decompiler emitted a counter loop (in sandbag's
`removeAllEffects`) whose non-null branch never advanced the index — the original
mutated the array as it iterated, an advance the decompiler dropped. Fired every frame via a `LINK_FRAMES`
listener, it hung the game loop. Fixed by `guard_loop_termination` in
`src/decompiler.rs` (see DEVELOPMENT §5.3) and verified by reading the regenerated
`Script.hx`. The engine harness now loads + spawns sandbag with no freeze; a
move-driven A/B (a freeze only manifests once effects exist and a state change
fires, which an idle pose doesn't trigger) is the remaining in-engine
confirmation.

**Open / deferred converter bugs surfaced during validation:**
- **IntervalTimer null callback (charge states).** Charge frame scripts emit an
  add-timer call whose third argument should be the timer *callback* but is instead
  the effects array, so the timer null-derefs when it fires (only on a charged
  smash). The `abc_parser` mis-resolved the SSF2 callback to the effects variable;
  the fix is in the AS3→hscript callback resolution.

---

## 6. Validation template — sandbag box geometry

The reference sweep, the template for the rest of the roster. Convert clean, then:

```
peptide harness \
  --project "$PWD/characters/sandbag/sandbag.fraytools" \
  --entity entities/Sandbag.entity --animation <anim> --frame <n> \
  --out-json /tmp/box.json --port 9222
cargo run -p ssf2_converter --features dev-tools --bin compare_boxes -- \
  --ssf2 ../ssf2-ssfs/sandbag.ssf \
  --char sandbag --json /tmp/box.json --tolerance 2.0
```

(`compare_boxes` is a `dev-tools`-gated diagnostic bin.)

FrayTools renders only **static** collision layers (hurt / item / body); hitboxes
are runtime script data and don't appear here (validated at runtime instead, §5).

**Result (current output):** every hurt / hit / body box converts sub-pixel
(0.000–0.001 px drift) across the move set (idle, jabs, tilts, strongs, aerials,
grab, specials) — 0 FAIL.

**Itembox — the one churny case.** The `itemBox` is the only routinely rotated
collision box (it's placed relative to the hand attachment point, with
`pivotY = height` so it rotates about the hand). Its rotated-affine anchor bake
(`entity_gen.rs`) was historically a ~3.7 px X-drift that **scaled with rotation**
(→ ~7 px at higher angles); the most recent sweep shows it resolved to <0.01 px in
current output. It is gameplay-noncritical (item pickup range, not hit/hurt
detection), but because it has churned repeatedly it is the **first box type to
re-check** if any geometry looks off — `peptide harness`'s `rendered_anchor` is
FrayTools' ground truth to bake against.

---

## 7. Pitfalls

- **Stale-output trap — verify you're debugging FRESH output.** The converter
  **merges into** whatever already exists in `characters/<id>/`; it does not wipe
  the dir first, so files from old converter versions (renamed-away paths like
  `scripts/Character/Script.hx` with a broken loop, old `library/sounds/*.ogg`)
  silently persist. Before trusting a runtime result, confirm the `.fra` you
  published came from a fresh regen (check `characters/<id>/library/scripts/<Pascal>/`
  mtimes; the dir should contain *only* current-converter paths). A full corpus
  purge once removed ~7,300 such files. *Prevention idea:* have the converter
  `library/`-scope clean each char's output dir at the start of a run.
- **Findex drift.** Engine builds renumber findices; some load-path ones survive
  across builds, others don't. **Resolve every called function by name**, never by
  a hardcoded findex; treat every findex in this doc as needing `fninfo`
  re-verification.
- **Wedged `hl` processes.** Orphaned `hl _conn.dat` procs from earlier harness
  boots occasionally wedge into uninterruptible sleep (`ps` STAT contains `U`,
  e.g. `UNE`); `kill -9` cannot reap them and they only clear on a full reboot.
  A wedged orphan does **not** block a new engine from launching, so the
  "kill prior instance" step gets a hard ~2-second budget — then move on. Don't
  loop / retry / wait it out.
- **Output is not the deliverable.** `characters/` is git-ignored, so converter
  output never shows in git — commit `src/` + docs changes, and re-derive output
  by re-running the converter (deterministic GUIDs make regen idempotent).
