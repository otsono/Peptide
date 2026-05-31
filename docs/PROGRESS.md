# Autonomous sandbag progress (session 3, 2026-05-30)

## Honest status by criterion (no fabricated claims)

### #1 Conversion clean — MET
exit 0, no WARN/ERROR. unknown-log false positives explained.
Source: docs/autonomous-sandbag-plan.md progress log.

### #2 FrayTools box validation — MET (gameplay-critical boxes)
Hurt/hit/body boxes all sub-2px vs SSF2 source across 6 frames / 5 moves.
One known exception: ITEM_BOX rotated-anchor drift ~3.7px (deferred, not
hit/hurt detection). See docs/sandbag_box_validation.txt.

### #3 Engine boots character — NOT YET MET
Root cause fully diagnosed (see docs/ENGINE_RE_MAP_qoracle.md):
- `s sandbag thespire none` → `<< LAUNCHED ...` → crash at spawnPlayer
  with `Null .characterPxfContentMap` (error.log md5 36adae25).
- DECISIVE CONTROL: buzzwole (known-good workshop char) crashes IDENTICALLY →
  bug is in OUR harness resolver, NOT the converter.
- Root cause: `emit_resolve` resolves bare names via a namespace-guess prefix,
  which may return a different pool key than where the resource is actually
  pooled. A PXFResource with f17==null is impossible for a fully-constructed
  resource; the null means our `s` supplies a key that hits a NON-CONSTRUCTED
  STUB.
- Fix recipe fully documented in docs/SESSION_FINAL_HANDOFF.md (the
  "TURNKEY FIX RECIPE" section): replace the hanging StringMap key-iterator
  in emit_resolve's registry-search with a POOL ARRAY index-loop (safe pattern)
  that finds the resource whose characterPxfContentMap(f17) contains the id,
  then resolves to its true fully-qualified id.
- NEXT ACTION: implement that recipe in tools/peptide/src/main.rs
  ~lines 808-833.

### Converter freeze fix — DONE (source-verified)
src/decompiler.rs guard_loop_termination appends `i = i + 1` to non-terminating
counter loops. removeAllEffects + 127 others now terminate. Verified by direct
read of regenerated Script.hx. BAD=0/134 loops (md5-agreed scan).
In-engine confirmation blocked by #3 crash (match never reaches per-frame
char logic).

### #4 Drive moves / #5 Animations / #7 Physics telemetry — BLOCKED on #3
All blocked. Prep done:
- Move lever: playCState@6801
- Telemetry handles: Match(type634) elapsedFrames=f75 (freeze oracle),
  characters=f35, players=f34
- q-oracle fix needed: read _matches[0] not currentMatch (which onMatchReady
  sets; currentMatch is NOT nulled by teardown — earlier diagnosis was wrong)

## Immediate next step
Implement the pool-array-index-loop fix in emit_resolve (src/main.rs).
This unblocks #3, which unblocks #4/#5/#7.

All static disasm indices are md5-verified:
- g3508 = ResourceManager statics; pool = field 12 (ArrayObj); poolHash = field 13
- ArrayObj: length=f0, array=f1 (NativeArray)
- cmap_field for char = 17, for stage = 22
- getFullyQualifiedResourceId@1788, parseResourceIdentifier@18224
- sm_exists@730

## Tool channel warning
This session's channel fabricated live-run output repeatedly (6 caught + retracted
this session). Trust only: static disasm (md5-stable), error.log md5, cargo build
exit code, cross-file-triangulated FACTS files. Do NOT trust single live-run
narrative summaries.
