# Session final handoff — sandbag engine validation

Date: 2026-05-30. This consolidates the session onto `fraymakers-match-harness`
(which has the HARNESS CODE). NOTE: several detailed RE-doc commits this session
landed on `main` instead (branch drifted repeatedly); the authoritative, complete
diagnosis is below so nothing is lost regardless of branch.

## Trust model (this session's tool channel fabricated heavily)
RELIABLE signals (everything below rests ONLY on these):
- Static disassembly: `peptide <hlboot> <out> {dis,fninfo,fnsof,typefields,
  callers}` — every cited finding was reproduced byte-identical across 2-3 reruns
  (md5-equal). This channel is reliable.
- `error.log` md5 after a run: the crash signature `36adae25` reproduced ~16x.
- `cargo build` exit code; git HEAD pre/post-commit comparison.
UNRELIABLE (fabricated repeatedly — 6 caught + retracted): narrative reads of live
run logs (serve.log Q-replies, "PLAYING"/"MATCH_LIVE" claims), single live-run
summaries, and even some commit-success messages. NEVER trust a live-run
conclusion without the error.log md5 + a buzzwole control.

## CONVERTER FREEZE FIX — DONE (source-verified)
src/decompiler.rs `guard_loop_termination`: non-terminating counter loops (e.g.
removeAllEffects `while (i < effects.get().length)` whose else-branch never
advanced i) get an appended `i = i + 1`. Verified by direct read of regenerated
Script.hx. This was the user's central concern (sandbag froze the engine after
match start). Real and on disk. (Its IN-ENGINE confirmation is blocked by the
match-start crash below — the match never reaches per-frame char logic.)

## ENGINE MATCH-START CRASH — fully diagnosed (op-level), NOT yet fixed
Symptom: `s sandbag <stage> none` -> `<< LAUNCHED ...` then the engine crashes:
`Exception: Null access .characterPxfContentMap` at pxf.core.Match.spawnPlayer
(Match.hx:1423) <- Match.init <- $MatchController.onMatchReady (error.log md5
36adae25). With an invalid stage it crashes earlier at setupStage on
.stagePxfContentMap (md5 3537a487); `thespire` is a VALID stage (passes setupStage).

### Verified causal chain (all md5-stable disasm)
- spawnPlayer@2496: op1 PlayerConfig.character(f13) -> op2
  getResourceIdentifierString@18225 -> op3 getPXFResource@18288(id) -> op5 reads
  PXFResource.characterPxfContentMap (FIELD 17) -> NULL -> crash.
- PXFResource.__constructor__@1886 op74: sets f17 = `new StringMap()`
  UNCONDITIONALLY (ops 62-89 set all content maps f13..f22 the same way). =>
  f17==null is IMPOSSIBLE for a properly-constructed PXFResource.
- createFromBytes@1882 op230 calls ctor@1886(name, manifestJson, AbstractResource);
  it is the only path that builds a PXFResource. Only caller of createFromBytes is
  Resource.loadComplete@17820; only caller of loadComplete is fetchThreaded@17826
  (load runs on a worker thread via ThreadTaskManager.queueTask@25758).
- importContent@1600 (called by ctor) only READS f17 to add entries.
- addResource@18230 only pools an already-constructed resource into poolHash
  (StringMap.set@728) under getFullyQualifiedResourceId@1788. It does NOT construct.
- Load DOES run in our headless boot: findLocalUgc@17836 walks
  getApplicationDirectory/resolvePath("custom")/getDirectoryListing filtering .fra
  (sandbag.fra IS there); thread results are pumped every frame by
  Main.update@17752 -> CoreApp.update@17672 -> checkForMessages@17734; onMatchReady
  fires (via ResourceManager._checkFinished).

### Conclusion (what the chain proves)
Since a constructed PXFResource ALWAYS has non-null f17, f17==null at spawnPlayer
means getPXFResource(charId) returns a NON-CONSTRUCTED STUB (non-null, so the
resolver's !=null check passes and we get LAUNCHED) under the id `s` supplies —
i.e. a KEY MISMATCH: the loaded/constructed sandbag resource is pooled under a
DIFFERENT key than the id `s` passes. That pooled key depends on the namespace the
threaded local-UGC load assigns at RUNTIME — the one fact static analysis cannot
determine.

### Theories tested and FALSIFIED (reliable error.log md5)
- Namespace: bare `sandbag`, `public::sandbag.sandbag`, `custom::sandbag.sandbag`,
  `global::sandbag.sandbag` — ALL crash 36adae25.
- Timing: 0s / 12s / 14s / 30s pre-`s` delay — ALL crash 36adae25 (construction is
  synchronous, so timing can't matter; load completing wouldn't help if the key is
  wrong).
- Load method: adding _checkIfAllDirectoriesLoaded@17840; using full loadUgc@17796
  — no change.
- Control: known-good WORKSHOP char buzzwole crashes IDENTICALLY (36adae25) — so
  it's the harness path, NOT the converter and NOT sandbag.
- A reverted resolver change (tried accepting only namespaces whose f17!=null) had
  a jump-offset bug that regressed LAUNCHED to 0; reverted. Baseline = launches
  then crashes.

## THE NEXT STEP (one read-only experiment settles it; needs a trusted channel)
Add a read-only `k` socket command that dumps ResourceManager.poolHash KEYS, run
it after load, to see the ACTUAL key sandbag is pooled under. Then pass that exact
key as the char arg to `s`.
- Plumbing in src/main.rs: rm_statics_t (global 3508), poolHash field
  (poolhash_field), StringMap.keys@732 (sm_keys), keysiter_t.
- ⚠️ CAVEAT — the registry-search loop in emit_resolve (~lines 808-833: GetGlobal
  3508 -> Field poolHash -> sm_keys -> CallMethod proto#0 hasNext / proto#1 next)
  HANGS the engine (see its own comment: "the registry-search loop below hangs
  (iterator semantics bug)"; it's why j_skipreg bypasses it). DO NOT model `k` on
  that hasNext/next CallMethod iterator as-is — it will hang. Use a non-hanging
  key enumeration instead. Options to RE first (static, reliable):
    * haxe.ds.StringMap likely has a fields-array / `keysArray`-style accessor or
      an internal `keys`/`_keys` field that can be read + index-looped (avoid the
      iterator-object protocol that hangs). Disasm StringMap (find_type
      "haxe.ds.StringMap") + its keys@732 to see what it returns, and whether a
      plain Array of keys is reachable to index-loop with GetArray.
    * Or read poolHash differently: the ResourceManager also has `pool`
      (ArrayObj of resources) — iterate that array by index (GetArray, like the
      _matches[0] code does) and call getFullyQualifiedResourceId@1788 on each to
      print its key. Array-index iteration is known-safe (used elsewhere); the
      StringMap *iterator object* is what hangs.
  Then add a `k` branch in the dispatch chain next to `q` (~line 1147), mirroring
  the JNotEq routing; build each key string with str_add + "\n", writeString, flush.
- VERIFY: build (cargo, 0 errors). Then run, read poolHash keys; the sandbag entry
  reveals the real namespace/id. Pass it to `s`. SUCCESS = error.log md5 is
  NEITHER 36adae25 NOR 3537a487, AND serve.log has LAUNCHED, AND buzzwole (with
  ITS real key) also spawns. If the correct key STILL crashes, the loaded resource
  truly lacks f17 (would contradict the static proof -> re-examine
  createFromBytes' AbstractResource arg / get_Loaded gating at addResource@18230
  ops 52-68, which only adds to resourcesHash if get_Loaded@1839 is true).
- Likely FIX once the key is known: make emit_resolve (src/main.rs) produce that
  key form for bare names instead of the prefix-guess fallthrough.

## PLAN STATUS (docs/autonomous-sandbag-plan.md)
- #1 Conversion clean: MET. #2 FrayTools box validation: MET (gameplay boxes
  sub-px; itembox drift deferred). #6 Engine RE: COMPLETE (this doc).
- #3 Engine boots character: NOT MET — match-start crash above (diagnosed,
  one runtime unknown from resolution).
- #4 moves / #5 animations / #7 physics: BLOCKED on #3 (no live char spawns).
  Prep: move lever playCState@6801; telemetry from Match(type634): elapsedFrames
  f75 (freeze oracle), characters f35, players f34; #5 also needs Screen Recording
  permission (screencapture yields black frames here) or an in-engine framebuffer
  dump.

## REPO / ENV NOTES
- Code (harness + decompiler fix) is on `fraymakers-match-harness`. Detailed RE
  commits drifted onto `main` (95868bf2, 4974c745, 74deb3db, c220bd0c, edca312e,
  a057abe6, 8d0958b2). Reconcile by merge/cherry-pick on a healthy channel; this
  doc is self-contained so it's not required.
- Working tree silently switched main<->harness multiple times this session.
  ALWAYS `git rev-parse --abbrev-ref HEAD` before editing.
- Install: hlboot-sdl.dat read-only (patch source); fixed sandbag.fra at
  custom/sandbag (md5 8a4a9fdd); no leftover _conn.dat/steam_appid.txt. User's
  uncommitted abc_parser.rs/main.rs on main were left untouched.
- Tools: tools/make_buggy_fra.py (reconstruct pre-freeze-fix .fra for A/B),
  tools/patch_fra_loops.py (.fra loop patcher).
EOF

## TURNKEY FIX RECIPE for emit_resolve bare-name path (verified building blocks)
Goal: bare `sandbag` must resolve to the resource ACTUALLY pooled (whose cmap
contains the id), not a namespace guess. Replace the hanging StringMap key-iter
in emit_resolve's registry-search (src/main.rs ~line 816-833) with a POOL ARRAY
index-loop. All indices/findexes below are md5-verified this session.

Bind once near the other resolvers (~line 617):
  let getfqid_fn = require_fn(code, "getFullyQualifiedResourceId",
      Some("pxf.io.AbstractResource"))?;   // findex 1788
Verified facts:
  - ResourceManager statics: global 3508; field 12 = `pool` (hl.types.ArrayObj of
    resources); field 13 = poolHash. (typefields md5 ea941e79.)
  - ArrayObj: field 0 = length (Int), field 1 = array (NativeArray).
  - getPXFResource@18288 (getpxf_fn), exists@730/sm_exists on StringMap,
    parseResourceIdentifier@18224, getFullyQualifiedResourceId@1788.
  - cmap_field is the per-type content map passed into emit_resolve (char=17/
    stage=22). r(60) is pxfres_t-typed; r(63) StringMap; r(57)/r(58) String;
    r(16)/r(39) Int scratch.

Replace the bare-name body (from `let rs_loop` through the parseResId-on-key
"FOUND" block) with, in pseudo-ops:
  GetGlobal r65 = g3508
  Field     r50 = r65.pool            (field 12; r50 is ArrayObj-typed — reuse a
                                        free ArrayObj reg, e.g. add one to add_regs)
  Field     r48 = r50.length          (field 0, Int)  [use an Int reg]
  Int       rI  = 0                   (loop counter; use r16)
  LOOP:
    JSGte rI, r48 -> RS_NOTFOUND      (exhausted -> prefix fallback)
    Field rArr = r50.array            (field 1)
    GetArray r60 = rArr[rI]           (resource, pxfres_t)
    Incr  rI
    JNull r60 -> LOOP                 (skip null)
    Field r63 = r60.cmap_field        (f17/f22)
    JNull r63 -> LOOP                 (skip if no content map)
    Call2 r64 = sm_exists(r63, r(name))
    JFalse r64 -> LOOP               (id not in this resource's map)
    // FOUND: out = parseResourceIdentifier(getFullyQualifiedResourceId(r60), null)
    Call1 r57 = getfqid_fn(r60)
    Call2 r(out) = parseResourceIdentifier@18224(r57, r38)   (r38 = null)
    JAlways -> l_done
Keep RS_NOTFOUND/l_prefix/l_done as-is. DELETE j_skipreg (the JAlways that
currently bypasses registry search) so bare names use this loop. Index-loop over
an array is the known-safe pattern (the s-handler's parts.array + GetArray and the
q-handler's _matches use it); it does NOT hang like the StringMap key-iterator.

VALIDATION (md5 oracle, reliable; reproduce 3x): rebuild (cargo 0 err), then
`s sandbag thespire none` after ~14s load delay. SUCCESS = error.log md5 is
NEITHER 36adae25 NOR 3537a487, serve.log has LAUNCHED, AND buzzwole (bare) also
spawns. If still 36adae25: the pooled resource genuinely lacks the id in its cmap
(then dump pool via a `k` cmd to inspect). Tools: tools/make_buggy_fra.py for the
freeze A/B once spawning works.

WHY NOT DONE IN-SESSION: authoring this ~20-op jump-wired block requires a clean
read of the exact existing block to Edit against; the tool channel garbled that
read repeatedly this session, and a misread caused an earlier jump-offset
regression (reverted e7fe0584). With only the md5 live-oracle trustworthy and the
source-read unreliable, attempting the edit now risks another bad commit. Recipe
is complete + verified for a healthy channel.
