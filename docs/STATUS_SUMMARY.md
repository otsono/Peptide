# Autonomous sandbag run — consolidated status

## DONE (verified, committed; 29 commits this effort)
- **FrayTools publish harness fixed** (cold-launch "No inspectable targets" race)
  in BOTH export-in-fraytools.js and harness.js (waitForTarget on /json/list).
- **Criterion #1 (conversion clean): MET.** sandbag converts exit-0, no
  WARN/ERROR. The big "unknown API" counts are false positives (calls are
  rewritten). Real residual unknowns are item/grab mechanics — revisit when
  engine testing is possible.
- **Criterion #2 (FrayTools layout match): MET for all gameplay-critical boxes.**
  compare_boxes vs SSF2: every hurt/hit/body box sub-pixel (0.000–0.001px).
  One LOW-severity exception: itembox rotated-anchor drift ~3.7px (scales with
  rotation) — full analysis + fix direction in docs/ITEMBOX_DRIFT.md, deferred
  (item pickup range, not hit/hurt detection).
- **Harness build-out:** FRAY_POST_READY_DELAY (let async load settle);
  loadUgc@17796 injection at READY (correct for a Steam-launched context).
- **Move-dispatch lever found (criterion #6 prep):** playCState@6801 — the
  internal "run a move without keypresses" entry point (Script.hx already uses it).

## BLOCKED — needs a user decision (criteria #3,#4,#5,#6)
**Hard root cause (sha-verified):** our control bridge runs the *patched*
bytecode via `./hl _conn.dat` directly, where the Steam API is NOT live —
engine stderr literally prints `[API loaded no]`. Steam UGC loading (the only
path that adds custom/ + workshop .fra to the ResourceManager pool) requires the
live Steam API. So custom characters never load → `getPXFResource` null →
`Match.spawnPlayer` crashes on `characterPxfContentMap`. Builtins (assets/data,
no Steam needed) spawn fine. Confirmed independent of: cwd (correct), timing
(50s wait fails identically), and the injected loadUgc (runs, but its async
file-load events never fire without the live API). buzzwole (clean external
char) fails identically → NOT our packaging.

**The decision (docs/ENGINE_BLOCKER.md):** the only known way to get BOTH our
injection AND live-Steam UGC loading is to temporarily swap the patched bytecode
into `hlboot-sdl.dat` and launch via Steam, restoring the original after
(reversible). That WRITES a Steam file, which the mandate's hard constraint
forbids without approval. Steam-file-free alternatives (lower confidence, need
RE): drive the already-running Steam-launched game via its in-engine console; or
make the headless process pass Steam's running-app check.

## ENVIRONMENT
- 4 `hl _conn.dat` processes wedged in uninterruptible sleep (unkillable; from
  earlier background boots). Harmless (random ports) but need a reboot to clear.
- Tool channel intermittently corrupted output this session (~8 catches via
  shasum/canary). All conclusions above are sha-verified.

## RESUME (once a path is chosen)
If "reversible hlboot swap via Steam" is approved: run.sh variant that backs up
hlboot-sdl.dat.orig, writes patched bytecode to hlboot-sdl.dat, `steam -applaunch
1420350` (or steam://run/1420350), waits for `[API loaded yes]` + READY, sends
`s sandbag ...`, restores .orig on exit. Then: verify sandbag spawns clean →
implement playCState move-driver (#6) → physics telemetry (#7) → animation
capture (#8) → iterate (#9). All the RE for those is done and noted in
docs/ENGINE_BLOCKER.md + memory.
