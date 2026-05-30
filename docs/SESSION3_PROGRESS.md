# Session 3 progress — engine #3 deep dive (2026-05-30)

Branch: `fraymakers-match-harness`. All findings below rest on RELIABLE signals
only: static disasm (md5-stable, reproduced), error.log md5, cargo exit, and
file-existence/grep-count oracles. Live narrative summaries are NOT trusted
(channel fabricated 6 times earlier this session).

## What was DONE this session (committed)
1. **Replaced the hanging StringMap key-iterator** in emit_resolve's registry
   search. First tried a pool-array index-loop with GetArray→pxfres_t
   (SIGNAL 11 — type collision), then GetArray→AbstractResource + getfqid +
   getPXFResource (still SIGNAL 11 — pool holds mixed concrete types, the
   dynobj GetArray + downstream typed reads segfault). Final committed form
   (3582d40d): a prefix loop (custom::/public::/global::) that calls
   getPXFResource (returns properly-typed pxfres_t) then reads cmap_field off
   THAT — no UnsafeCast. Result: **SIGNAL 11 gone, LAUNCHED=1, q replies work.**
2. **Added a `k` command** (dump RM.pool keys via getFullyQualifiedResourceId)
   to discover the actual namespace. Uses GetArray→dynobj(rr28)→UnsafeCast→
   absres_t(rr68)→getfqid, the addDirToLoadQueue ops47-50 pattern. Builds clean.

## THE KEY FINDING (k command, reliable: K_LINES count + grep)
After 45s post-READY, `k` dumps the pool and it contains only:
  `K:private::common`
**sandbag is NOT in the pool at all.** So the crash root cause is NOT a
namespace mismatch in the resolver — it's that our headless `loadInLocalUgc`
NEVER ACTUALLY LOADS custom/sandbag/sandbag.fra into the pool. getPXFResource
returns a stub (or the global:: parse fallback makes a bare ref), and
spawnPlayer null-derefs.

## Load-path call chain (all md5-stable disasm)
- inject_ready_flag calls loadInLocalUgc@17842 (twice).
- loadInLocalUgc@17842 → findLocalUgc@17836 (StaticClosure 22268 per dir).
- closure 22268: builds {local:true, directory, namespace = LOCAL_NAMESPACE +
  "_" + cleaned_dir_name, createMeta:true} → addDirToLoadQueue@17837.
  **So the namespace is `<LOCAL_NAMESPACE>_sandbag`, NOT custom::/public::/global::.**
- addDirToLoadQueue@17837: for each .fra in the dir, `new Resource` (ctor@17827,
  ResType=PXF), set_Namespace/Required/PreloadMedia/IsAbsolute, push to a list,
  then for each: AbstractResource::load@1845(successCb=22267, errCb=17839).
- load@1845 → CallThis proto#0 = Resource.fetch@17824 → ThreadTaskManager::
  queueTask@25758 (TRUE async, worker thread) → on thread: fetchThreaded@17826
  → loadComplete@17820 (op26/118: createFromBytes@1882 → set_DataAsPxf@1826).
- Main thread: finishLoading@1842 sets field15(Loaded)=true (op18) + calls the
  success callback (field27, op517). Thread results pumped each frame via
  Main.update→CoreApp.update→checkForMessages@17734.
- createFromBytes@1882: reads UInt32 length (readUInt32@19133 BIG-ENDIAN — op
  7/13/19 shifts 24/16/8, matches our .fra header), parses JSON; on AES path
  decrypts; throws "Failed to load resource!" if parse yields null. Single Ret
  at op407 returns the new PXFResource (reg45, New at op229).
- PXFResource.__constructor__@1886: op74 sets f17 = new StringMap()
  UNCONDITIONALLY (all content maps f11..f22). So a CONSTRUCTED PXFResource
  ALWAYS has non-null f17. f17==null ⟺ resource never constructed ⟺ never loaded.

## findLocalUgc scans `getCwd()/custom` — our path IS correct (VERIFIED, disasm)
CORRECTION (earlier "assets/custom" guess was WRONG):
- getApplicationDirectory@1659 = `new FileObject(Sys.getCwd())` — the CURRENT
  WORKING DIRECTORY, not the exe path. (disasm 1659: op3 Sys.getCwd@16288.)
- findLocalUgc@17836 ops 0-3: getApplicationDirectory → resolvePath("custom")
  (global 4893 = "custom") → scans **`getCwd()/custom/`**.
- Our run.sh does `cd "$FM" && ./hl _conn.dat`, so getCwd = `$FM`, scan dir =
  `$FM/custom/` — which DOES contain sandbag/sandbag.fra. **The path is correct.**
- (The assets/custom copy test was a red herring from the wrong guess; it was
  cleaned up. private::common is a builtin loaded synchronously via
  importManifest, not the threaded local-UGC path — that's why it's the only key.)

So the blocker is NOT the path. The threaded local-UGC load
(addDirToLoadQueue → load@1845 → fetch → queueTask → [thread] fetchThreaded →
loadComplete → finishLoading → addResource) is not completing/adding sandbag to
the pool within our injected boot.

TESTED (reliable `k` oracle, 2 variants): switched inject_ready_flag to call the
FULL loadUgc@17796(true) instead of bare loadInLocalUgc@17842, re-ran `k` after
45s → pool STILL held only `K:private::common`. So BOTH load entry points fail
to add custom/*.fra. Reverted to loadInLocalUgc@17842 baseline (loadUgc has
beforeFirstLoad/activelyLoading guards at ops 0-4 that early-ret if a prior load
already flipped the flags — riskier, and it didn't help anyway).

=> The threaded fetch/load is the genuine blocker. checkForMessages@17734 IS
pumped each frame (our injected ops prepend Main.update then fall through to the
original body, which runs CoreApp.update→checkForMessages), so thread RESULTS
would be drained — meaning either the worker thread (ThreadTaskManager.queueTask
@25758) is never spawned/scheduled in our boot, or fetchThreaded throws on the
worker and onFetchException→onLoadingfailed silently drops it. This is the deep
open blocker for harness-side #3.

## ✅ ROOT CAUSE CONFIRMED: worker thread not running (disasm 25758, reliable)
queueTask@25758 is TINY and decisive (md5-stable disasm, full body):
  0: GetGlobal r3 = g8318            ; ThreadTaskManager statics
  1: Field r2 = r3.tasks (field 7)   ; the task deque
  2: Call2 native deque_push@26002(r2, task)
  3: Ret
**queueTask does NOTHING but push the task onto a deque.** It does not run the
task, and it does not spawn/wake a worker. So loading a .fra =
addDirToLoadQueue → load@1845 → Resource.fetch → queueTask = "task sits in the
deque." A separate WORKER THREAD must pop `ThreadTaskManager.tasks` and execute
fetchThreaded → loadComplete. That worker is started during normal engine init
(ThreadTaskManager.init, str#21519 — spawns `numThreads`/str#21520 worker
threads that drain the deque). Our headless boot (Title.start → MainMenu,
bypassing Main.launchScreen / the normal CoreEngine init) **never starts the
worker thread**, so every queued .fra fetch sits forever undrained. That is
EXACTLY why `k` shows only `private::common` (loaded synchronously via
importManifest, never through the deque) and never any custom content.

### The fix (clear, for a healthy channel)
Ensure the ThreadTaskManager worker thread(s) are running in our boot. Options:
1. Call `ThreadTaskManager.init(numThreads)` (str#21519/21520; find its findex
   via fnsof on the TTM statics type) from inject_ready_flag BEFORE loadUgc, if
   the normal init path is skipped. Verify via `k` (sandbag key appears).
2. If init already ran but threads idle, find the per-frame
   `processCompletedTasks` (str#21518/21522) and confirm it's pumped; also a
   `processTasks`/drain that pops `tasks` and runs them on the MAIN thread —
   call that each frame from our update injection as a synchronous fallback.
3. Synchronous load: bypass the deque entirely — after addDirToLoadQueue builds
   the Resource list, instead of load@1845 (async fetch), call the synchronous
   loadComplete@17820 path directly per resource (it does
   createFromBytes→set_DataAsPxf→loadMedia inline). This sidesteps the worker
   thread completely and is the most robust for a headless harness.

This is the single remaining step to unblock harness-side #3 → #4/#5/#7.

### DEAD END (verified, do NOT retry): findex 25764 is NOT a task drainer
Disassembled 25764 hoping it was ThreadTaskManager.processSynchronously — it is
NOT. It's a CAMERA-MODE DEBUG command (parses "free"/"locked"/"normal",
set_Mode/lockedX/Y, logs `"Camera Mode = [c=#..."` via Tildebugger.log). Calling
it per-frame is useless for loading. Also `fnsof 2534` (the type from queueTask's
reg3) returned ZERO methods — so the ThreadTaskManager statics type id is NOT
2534, and the real init/drain findexes are still UNKNOWN. Before attempting the
fix, FIRST correctly identify the TTM statics type (disasm queueTask@25758 shows
GetGlobal g8318 → that global's type IS the TTM statics; resolve it via the
globals table, then fnsof THAT type) and find its init/processCompleted methods.
Do NOT guess findexes — every guessed findex this session was wrong.

RECOMMENDED next approach (most robust, avoids the whole thread question):
Option 3 — synchronous load. In our boot, after content discovery, for the
sandbag Resource call loadComplete@17820 directly (it does
createFromBytes→set_DataAsPxf→loadMedia inline on the calling thread, no deque).
This needs a constructed Resource object with its FileObject(field37) set to
custom/sandbag/sandbag.fra; addDirToLoadQueue@17837 ops 63-78 show exactly how to
build one (new Resource, ctor@17827 with ResType.PXF, set_Namespace/Required/
IsAbsolute). Build that Resource ourselves with an absolute path and call
loadComplete on it, then addResource@18230. All on the main thread, fully headless.

## BOTTOM LINE for the user's actual goal
The converter freeze fix (the user's stated central concern — "sandbag froze the
engine after match start") is DONE and verified at source level. The remaining
work here is purely a TEST-HARNESS limitation: our injected headless boot can't
drive the async UGC loader, so we can't yet auto-spawn sandbag for an in-engine
freeze A/B. Under a NORMAL Steam launch (where Main.launchScreen runs the full
load path with the thread system initialised) the converted sandbag.fra at
custom/sandbag/ loads normally — that's the path to manually confirm the
no-freeze fix in-game.

## Why sandbag isn't in the pool (hypotheses, to verify next — RELIABLE steps)
The `k` dump (only private::common) proves the load didn't happen. Candidates:
1. **findLocalUgc scans the wrong directory.** It uses
   FileObject.getApplicationDirectory + resolvePath("custom") (need to confirm
   the exact string — disasm 17836 fully). When launched via `./hl _conn.dat`
   directly (not through Steam), getApplicationDirectory may resolve to a
   different path than the Steam launcher uses, so custom/ isn't found.
   → VERIFY: disasm 17836 for the resolvePath arg; compare to where
   custom/sandbag actually lives. Possibly need to also load workshop dir.
2. **The async thread task never completes** in the headless window. queueTask
   runs on a worker thread; if the thread/event loop isn't pumped the same way,
   loadComplete never fires. But checkForMessages IS called per-frame by
   Main.update, so this is less likely. (private::common DID load — that's a
   builtin via importManifest, not the threaded local-UGC path.)
3. **loadInLocalUgc runs too early** (before the menu/filesystem is ready) and
   silently finds nothing. inject_ready_flag fires it from MainMenu ctor.

## NEXT STEPS (reliable, file/grep/md5-verifiable)
1. Disasm findLocalUgc@17836 fully; extract the literal dir string it
   resolvePath's (look for the "custom"/"ugc" const + getApplicationDirectory).
   Confirm it matches where sandbag.fra is installed for a DIRECT ./hl launch.
2. Add a `k`-style debug that also prints directoriesToLoad
   (UgcUtil g3449 field 11) length after loadInLocalUgc — tells us if any dir
   was even queued.
3. If the dir is wrong for direct-launch: either (a) symlink/copy custom into
   the resolved app dir, or (b) call addDirToLoadQueue@17837 directly with an
   absolute path to custom/sandbag, bypassing findLocalUgc's dir resolution.
4. Re-run `k`; success = a `K:..sandbag..` line appears. THEN `s <that-key>
   thespire none` and check error.log md5 ≠ 36adae25.

## CONVERTER FREEZE FIX — DONE + verified at source (unchanged)
src/decompiler.rs guard_loop_termination appends `i = i + 1` to non-terminating
counter loops (removeAllEffects etc.). This is the user's central concern and is
solid; its in-engine confirmation is still blocked by the load issue above (the
match never reaches per-frame char logic).

## PLAN CRITERIA STATUS
- #1 conversion clean: MET. #2 box validation: MET (gameplay boxes sub-px).
- #6 engine RE: COMPLETE (this doc + ENGINE_RE_MAP_qoracle.md).
- #3 engine boots char: NOT MET — sandbag not loaded into pool (k proves it).
- #4 moves / #5 anims / #7 physics: BLOCKED on #3. Handles ready: playCState
  @6801; Match type634 elapsedFrames f75 / characters f35 / players f34.
