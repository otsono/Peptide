# Fraymakers harness — audit (what exists vs. what's needed)

Session goal (reprioritized): build the Fraymakers-side harness so it can validate a
port end-to-end — **load → drive inputs via internal engine functions → measure
physics/state → assert** — then scale to all 47 characters.

## What exists (verified this session)

- **Patcher `peptide`** (`src/main.rs`, 1451 lines): parses `hlboot-sdl.dat` via
  `hlbc`, injects a per-frame dispatch block into `fraymakers.Main.update`, writes a
  patched `_conn.dat`. Resolves most engine fns **by name** (`require_fn`) — robust to
  findex drift. Inspection modes: `dis/fninfo/fnsof/typefields/callers/whoref/strgrep/inspect`.
  Invocation: `peptide <in.dat> <out.dat> <mode> <arg>` (output on stderr).
- **`peptide-bridge`** (`src/bin/peptide-bridge.rs`): loopback TCP bridge (serve/send).
- **Command surface today:** `p` ping · `c` console (h2d.Console.runCommand) · `s`
  start match (real FraymakersMode.startMatch — **works for builtins**) · `q` match-live
  query · `k` pool-key dump. Plus a `cmap-check` prefix probe.
- **`inject_ready_flag`**: at READY fires `ThreadTaskManager.init@25781` (uncommitted) +
  `loadInLocalUgc@17842` to attempt custom-content load.
- **`run.sh`**: recreates `_conn.dat`+`steam_appid.txt`, launches `./hl _conn.dat`,
  bridges one command, deletes added files. Never modifies `hlboot-sdl.dat`.

## Findex-drift finding (IMPORTANT)

Docs cite raw findexes. Most load-path ones still match this build (verified:
25781=ThreadTaskManager.init, 25758=queueTask, 26003=threadLoop, 17842=loadInLocalUgc,
17796=loadUgc, 18228=importManifest). **But `playCState@6801` is STALE** — 6801 here is
`noDirectionalInfluenceBehaviorCallback`. Rule: **resolve every called fn by name**; treat
all doc findexes as needing re-verification via `fninfo`.

## The two structural blockers (both must be addressed for scale)

1. **Custom content won't load headless.** `./hl _conn.dat` gets `[API loaded no]` —
   Steam API not live → UGC async pipeline never completes → `poolHash` empty for custom
   chars → `getPXFResource` null → `spawnPlayer` crash. Builtins (assets/data) load via a
   different path and spawn fine. (Under investigation: sync-load path vs. spawning the
   UGC worker thread.)
2. **Headless `./hl` wedges on exit.** 10+ orphaned `hl _conn.dat` procs in *uninterruptible*
   sleep (UNE/UE, 7–15h old) — stuck on an SDL/GPU syscall; `kill -9` can't reap them. Every
   boot risks a permanent zombie → headless iteration is fragile at scale.
   **Mitigation idea:** inject `Sys.exit(0)` after the diagnostic/validation completes so the
   process terminates cleanly instead of spinning in the render loop.

## What's needed (to build)

- `m <move>` — drive a move on the live player Character via its internal state API (NOT
  keypress). Needs: Character type, the move-trigger method (real findex), CState enum, and
  the walk currentMatch→Match→player0→Character.
- `t` — physics/state telemetry per tick (x/y, velocity, state/anim id, %/damage, facing).
- frame capture — per-frame image (OS `screencapture` or engine framebuffer) after seeking.
- assert/runner — compare measured state to expected; emit PASS/FAIL per character.

Build & test against a **builtin** (commandervideo) first — that path loads, so move/
telemetry/capture are unblocked by blocker #1.
