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

## findLocalUgc scans `<appdir>/assets/custom` (VERIFIED, disasm 17836)
getApplicationDirectory@1659 = `directory(sys_exe_path())`. Under our direct
`cd $FM && ./hl _conn.dat` launch, sys_exe_path = `$FM/hl`, so appdir = `$FM/`.
findLocalUgc@17836 ops 19-24: getApplicationDirectory → resolvePath("assets") →
resolvePath("custom") → localUgcDir. So it scans **`$FM/assets/custom/`**, NOT
`$FM/custom/`. Our content is installed at `$FM/custom/sandbag/`.

TEST (reliable, k-line grep): copied sandbag.fra+meta.json to
`$FM/assets/custom/sandbag/` and re-ran `k` after 45s. Pool STILL contained only
`K:private::common` — sandbag did NOT load even from the correct scan dir. So
the path mismatch is real but NOT the sole blocker: the load is additionally
async/gated (the threaded fetch→loadComplete→addResource never completed for our
dir in the window, OR findLocalUgc's per-dir logic — subscriptions check at op
27, isDirectory gates — skipped it). assets/custom test placement was cleaned up.

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
