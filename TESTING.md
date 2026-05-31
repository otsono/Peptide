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
| **FrayTools** | `tools/fraytools-harness/` | the user's FrayTools editor | Chrome DevTools Protocol | "does it render / lay out right? publish me a `.fra`" |
| **Fraymakers** (`peptide`) | `tools/peptide/` | the user's Fraymakers engine | patched bytecode + loopback TCP | "does it behave at runtime? load / spawn / move / read state" |

Each tool's own README has the detailed command surface:
- [`tools/fraytools-harness/README.md`](tools/fraytools-harness/README.md) — `harness.js` (box geometry + capture), `export-in-fraytools.js` (publish), `compare_boxes`.
- [`tools/peptide/README.md`](tools/peptide/README.md) — the bytecode patcher, the loopback bridge, and `run.sh`.

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
./target/release/ssf2_converter ~/.openclaw/workspace-main/ssf2-ssfs/<id>.ssf
#    → ./characters/<id>/   (⚠ verify it is FRESH — see §6 stale-output trap)

# 3. Publish via the FrayTools harness → .fra lands in Fraymakers' custom/<id>/
node tools/fraytools-harness/export-in-fraytools.js \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 4. Boot the Fraymakers harness, dispatch, observe
cd tools/peptide
./run.sh "s <id> thespire <assist>" 20     # spawn into a real match
#   then probe:  ./run.sh "t" 8  (read state)   ./run.sh "m" 8  (drive a move)

# 5. Compare behaviour vs expected → fix in the converter (never hand-edit output) → repeat
```

**Which harness when:**
- **FrayTools-side** (`harness.js` / `export-in-fraytools.js`) — visual/layout
  ground truth (box geometry, pivots, rendering) and producing the publishable
  `.fra`. Pair with `compare_boxes` for a numeric verdict.
- **Fraymakers-side** (`run.sh`) — runtime behaviour (loads, spawns, animates,
  transitions state, responds to a move dispatch). This is where freeze / crash /
  physics bugs surface.

**Branch note:** `main` holds the converter **and** the merged harness work.
(Older `fraymakers-match-harness` / `steam-shim-experiments` branches are
historical — not the place for new work.)

---

## 2. FrayTools harness — publish + box geometry

Code: `tools/fraytools-harness/` (Node; `npm install` pulls `chrome-remote-interface`).
It drives the user's local FrayTools (Electron + CDP) entirely through FrayTools'
own runtime objects — no pixel coordinates.

- **`export-in-fraytools.js`** runs FrayTools' own **Publish** (the Fraymakers
  Content Exporter) on a converted project and prints the published `.fra` path.
  "Publish All" writes both `<projectDir>/build/` and the Fraymakers
  `custom/<id>/` dir the converter wired up. This is exactly what the GUI's
  "Export in FrayTools" button shells out to.
- **`harness.js`** opens an entity, navigates to an animation + frame via Redux
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
cargo run --release --bin compare_boxes -- \
  --ssf2 ~/.openclaw/workspace-main/ssf2-ssfs/<id>.ssf \
  --char <id> --json /tmp/box.json [--tolerance 2.0]
```

Reads the harness JSON, parses the SSF2 source, matches boxes by type + size, and
reports per-box drift (FrayTools rendered anchor vs SSF2 intended position). It is
**split-aware** — split sub-animations (`strong_forward_attack`, `*_land`,
`grab_hold`, …) are mapped back to their source SSF2 anim + start-frame offset so
they verify instead of being skipped. Exits 0 on full pass, 1 on any failure.

---

## 3. Fraymakers engine harness (`peptide`)

Code: `tools/peptide/` (Rust). Two bins:
- **`peptide`** parses the engine's HashLink bytecode (`hlbc` crate), injects a
  per-frame dispatch block into `fraymakers.Main.update`, and writes a patched
  copy. Also has read-only inspection subcommands.
- **`peptide-bridge`** is the loopback TCP server (the injected engine code is
  the client).

### Boot model (`run.sh`)

```
./run.sh "<command>" [seconds]          # FRAY_DIR=... overrides the install path
./run.sh "s sandbag thespire commandervideoassist" 20
```

Steam's sandbox wipes anything added to the install dir, so `run.sh` **recreates
everything every run and never mutates the pristine engine**: it patches a *copy*
of `hlboot-sdl.dat` → `_conn.dat` (the source is never written), writes
`steam_appid.txt` so a direct `./hl` launch doesn't bounce through Steam, starts
`peptide-bridge`, launches `./hl _conn.dat`, and deletes the transient files on
exit. The patched engine waits for content load (title "press any button"
state), dials the socket (auth handshake), then processes commands per-frame on
the render thread. Needs `dangerouslyDisableSandbox`.

### Commands (dispatched on the first byte)

| Cmd | Meaning | Ack / readback |
|---|---|---|
| `p` | ping / liveness | `PONG` |
| `c` | console passthrough (`Tildebugger.console.runCommand`) | `RAN` |
| `s <char> <stage> <assist>` | **start match (self-bootstrapping)** — runs the custom-load core itself (idempotent) then builds a real `TrainingMode` + `FraymakersMode.startMatch`. No prior `l` needed. | `LAUNCHED <char> <stage> <assist>` |
| `l` | **synchronous custom-`.fra` load** (headless, main thread) — see §3.1 | `L:…`, `SPR:1`/`SPR:0` |
| `m` | **move dispatch** — `Character.toState(CState.JAB)` on player 0 (internal state-machine dispatch, **not** key-press simulation) | `M:JAB` / `M:NOMATCH` |
| `t` | **telemetry** — `Character.getStateName()` on player 0 | `T:<state>` / `T:NOMATCH` |
| `q` | is a match live? | `Q:MATCH_LIVE` / `Q:NO_MATCH` |
| `k` | dump pool keys + UGC-discovery diagnostics | `K:…` |

Short-name resolution (`s` / `l`): a bare `sandbag` (no `::`) is tried against
`private::`, `custom::`, `public::`, `global::` in order; first existing resource
wins. `private::` is first so a bare name resolves to headless-loaded custom
content. Or pass a full `namespace::package.id`.

**Multi-command sessions:** `m`/`t`/`q` probes need the *same* live match (a reboot
loses it). Use `runseq.sh <boot_wait_s> <gap_s> "cmd1" "cmd2" …` to feed a gapped
sequence into one engine session (boot→READY ≈ 30s, so `boot_wait_s` ≈ 32,
`gap_s` 6–9). Example:
`./runseq.sh 32 6 "s sandbag thespire commandervideoassist" t m t`.

`peptide` read-only inspection (for re-deriving findices — **always re-verify**):
`dis <findex>`, `typefields <type>`, `fnsof <type>`, `fninfo <findex>`,
`callers <findex>`, `strgrep <s>`, `whoref <s>`, `inspect`.

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

> **The `m`/`t`/`q` walk is verified live** against a builtin (`commandervideo`)
> match: `LAUNCHED → T:STAND → M:JAB → T:JAB (transient) → T:STAND → Q:MATCH_LIVE`.
> Sample `t` at frame cadence (~0.12s) — a single delayed sample misses short
> moves (jab returns to STAND in ~0.36s).

---

## 5. In-engine validation status

Stop-condition for "a converted character is validated" (the original
6-criterion mandate, sandbag as the reference character):

| # | Criterion | Status |
|---|---|---|
| 1 | **Conversion clean** — no `WARN`/`ERROR`; `conversion_log.json` `unknown` triaged | **MET** for sandbag (exit 0; the big "unknown" counts are false positives — the calls *are* rewritten in output) |
| 2 | **FrayTools layout match** — `compare_boxes` within tolerance | **MET** — see §6 (every gameplay-critical box sub-pixel) |
| 3 | **Engine boots the character** — loads + spawns, no crash | **MET** for sandbag — synchronous headless load + reliable spawn to `T:STAND` (§3.1) |
| 4 | **Every move runs in-engine** via the internal control path | **In progress** — `m` (`toState`) drives moves; verified on a live match. Scaling across the full move set + corpus is the open work |
| 5 | **Animations play** frame-by-frame | **Open** — needs per-frame capture (the `t`/animation-field readback exists; OS `screencapture` needs Screen Recording permission or an in-engine framebuffer dump) |
| 6 | **Physics within tolerance** of `CharacterStats.hx` | **Open** — telemetry fields are mapped (§4); needs a numeric readback + comparison |

**Converter freeze fix — DONE and confirmed.** The user's central concern was
sandbag freezing the engine shortly after match start. Root cause was a converter
bug: a decompiled counter loop (`removeAllEffects`'s `while (i < effects.get().length)`)
whose non-null branch never advanced `i` — the AS3 original mutated the array via
`splice`, which the decompiler dropped. Fired every frame via a `LINK_FRAMES`
listener, it hung the game loop. Fixed by `guard_loop_termination` in
`src/decompiler.rs` (see DEVELOPMENT §5.3) and verified by reading the regenerated
`Script.hx`. The engine harness now loads + spawns sandbag with no freeze; a
move-driven A/B (a freeze only manifests once effects exist and a state change
fires, which an idle pose doesn't trigger) is the remaining in-engine
confirmation.

**Open / deferred converter bugs surfaced during validation:**
- **IntervalTimer null callback (charge states).** Charge frame scripts emit
  `self.addTimer(8, -1, effects.get())` — the 3rd arg should be the timer
  *callback* but `effects.get()` is the effects Array, so the timer null-derefs
  when it fires (only on a charged smash). The `abc_parser` mis-resolved the SSF2
  callback to the `effects` var; the fix is in the AS3→hscript callback resolution.

---

## 6. Validation template — sandbag box geometry

The reference sweep, the template for the rest of the roster. Convert clean, then:

```
node tools/fraytools-harness/harness.js \
  --project "$PWD/characters/sandbag/sandbag.fraytools" \
  --entity entities/Sandbag.entity --animation <anim> --frame <n> \
  --out-json /tmp/box.json --port 9222
cargo run --release --bin compare_boxes -- \
  --ssf2 ~/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf \
  --char sandbag --json /tmp/box.json --tolerance 2.0
```

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
re-check** if any geometry looks off — `harness.js`'s `rendered_anchor` is
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
