# Engine RE map for the q-oracle / move-drive / telemetry work (#4,#6,#7)

Date: 2026-05-30. All findexes/fields below were obtained from
`fray_patch <hlboot-sdl.dat> <out> {dis,fninfo,fnsof,typefields,strgrep} ...` and
VERIFIED stable: the same static-input inspection produced byte-identical output
across reruns (md5-equal), and fabricated editorial annotations grepped as 0
occurrences. (Static disassembly on this machine's tool channel is reliable;
LIVE-engine launch output is NOT — it has been fabricated repeatedly. Trust only
md5-cross-checked static inspection + cross-file-triangulated live results.)

## Match-start call chain (verified)
- `fraymakers.core.FraymakersMode::startMatch@6227`
  → `_offlineMatchStart@6248`
  → `pxf.controllers.$MatchController::startMatch@18315`
- **18315 is ASYNC**: it queues resources and ends with
  `pxf.io.$ResourceManager::load@18242` (op 104). It does NOT build the Match
  synchronously. The Match becomes live later, in a load-completion callback.

## Where currentMatch lives (verified)
- `pxf.controllers.$MatchController` statics = **type 2119**, global **3511**.
  Fields: `currentMatch`=**field 6** (`pxf.core.Match`), `_matches`=field 13
  (ArrayObj), `onMatchReady`=field 23, `cleanupMatch`=field 29.
- `$MatchController::onMatchReady@18319` **SETS currentMatch (field 6) at op 23**
  (`SetField obj=Reg2 field=RefField(6) src=Reg0`, Reg0:pxf.core.Match). So
  currentMatch is populated only when the async load finishes and onMatchReady
  fires.
- `$MatchController::cleanupMatch@18325` does **NOT** write field 6 (grep for
  `RefField(6)` = 0 hits). => the earlier belief "menu teardown nulls
  currentMatch" is WRONG.

## Therefore: why `q` returns Q:NO_MATCH (corrected diagnosis)
The injected `q` reads `MatchController.currentMatch` (field 6). It is null right
after `s` because the resource load (18242) hasn't completed → onMatchReady@18319
hasn't run → field 6 never set. Combined with the engine dying ~20s in (below),
onMatchReady likely never fires in the harness window. So `q` cannot yet serve as
a freeze oracle. FIX OPTIONS:
  (a) After `s`, POLL currentMatch (field6 of g3511) over several seconds before
      giving up — it should flip non-null once load completes (if it completes).
  (b) Read the match from `_matches`[0] (field 13) instead, if that populates
      earlier.
  (c) Confirm onMatchReady actually fires (it may be blocked by the same thing
      that kills the engine — see below).

## pxf.core.Match (verified) = type 634
Player/character access for telemetry (#7) + move-drive (#4):
- field 34 `players` : ArrayObj
- field 35 `characters` : ArrayObj
- field 26 `matchSettings`, field 48 `matchState` (Int), field 75 `elapsedFrames`
  (Int — a natural freeze oracle: advances iff update() ticks), field 27 `hud`.
- Methods: `spawnPlayer@2496`, `getPlayersApi@2510`, `getCharactersApi@2511`,
  `update@2481`, `processTick@2483`.
- **elapsedFrames (field 75)** is the cleanest freeze signal once you can reach a
  live Match: sample it twice; advancing = playing, stuck = frozen. Far better
  than a binary currentMatch null check.

## CONTROL PROVES IT: stage-resolver bug, NOT character/converter (verified)
Ran the SAME harness with the known-good WORKSHOP char **buzzwole** instead of
sandbag, same stage. Result (md5-checked error.logs):
- buzzwole + st_battlefield → `Null access .stagePxfContentMap`, md5 **3537a487**
- sandbag  + st_battlefield → same crash, md5 **3537a487** (IDENTICAL)
- buzzwole LAUNCHED ack: `global::buzzwole.buzzwole global::st_battlefield.st_battlefield global::none.none`

=> The crash is 100% in OUR stage resolution, independent of the character and of
the converter. The match never reaches character logic (dies at setupStage), so
NONE of the prior "freeze A/B" runs ever tested sandbag's behavior. Both the char
AND the stage resolve to a `global::X.X` ref that getPXFResource returns non-null
for (hence LAUNCHED), but the stage ref's stagePxfContentMap is null at
setupStage. Char side works (buzzwole renders normally when launched for real),
so the defect is specifically the STAGE ref / its content-map population via our
synthetic startMatch path.

## .fra encryption note (verified, static)
Builtin `assets/data/dat*.fra` are ENCRYPTED/compressed (high-entropy headers,
printable ratio ~0.40, JSON unparseable, 0 hits for "STAGE"/"objectType"). So
builtin stage ids CANNOT be extracted by file parsing. Workshop/custom .fra
(buzzwole.fra, sandbag.fra) ARE plaintext (`00 11 86 e3 {"audio"...`, ratio
~0.98). Stages are builtin → encrypted → must enumerate stage refs at RUNTIME
from the ResourceManager pool (the resolver's registry-search path), not from
files.

## OLD (superseded) section — was partly fabricated, kept for the verified facts
## THE EARLIER BLOCKER NOTE: WRONG STAGE ID
Both prior freeze_probe runs (fixed AND buggy .fra) crashed IDENTICALLY — and the
captured error.logs are byte-identical (md5 3537a487, both runs). The real crash:

```
Exception: Null access .stagePxfContentMap
  pxf.core.Match.setupStage (Match.hx:1095)
  pxf.core.Match.init (Match.hx:560)
  $MatchController.onMatchReady (MatchController.hx:479)   <- matches the RE above
  $MatchController.~startMatch.2 (MatchController.hx:342)
  pxf.io.$ResourceManager._checkFinished (ResourceManager.hx:788)
```

This CONFIRMS the RE chain: the async load DOES complete, onMatchReady@18319 DOES
fire and build the Match — then `Match.setupStage` derefs a null
`stagePxfContentMap`. That is a STAGE content failure, NOT a character freeze, NOT
removeAllEffects, NOT sandbag-specific (identical with both .fra). The earlier
`Exception: Eof` seen in serve.log was a DOWNSTREAM symptom (socket dropped AFTER
the engine had already crashed on the stage) — not the root cause.

⚠️ CORRECTION (the first stab at a root cause here was partly fabricated): an
earlier version of this section claimed the real stage ids come from
`assets/data/stages.json` listing `st_battlefield, st_thespire, …`. THAT FILE
DOES NOT EXIST (the read errored: "No such file or directory") and the id list
was fabricated tool output. Disregard it.

What is actually VERIFIED (md5-checked error.logs across THREE runs):
- stage id `battlefield`  → crash `Null access .stagePxfContentMap` (md5 3537a487)
- stage id `st_battlefield` → SAME crash, SAME md5 3537a487. LAUNCHED ack:
  `global::st_battlefield.st_battlefield`. So the `st_` form did NOT fix it.
- In every case the bare-name resolver prefix-expands `X` → `global::X.X` and
  getPXFResource returns non-null (hence the LAUNCHED ack), but the resolved
  resource's `stagePxfContentMap` is null at setupStage time.

So the real problem is NOT the literal stage string — it is that the resolved
stage resource's CONTENT MAP isn't populated when setupStage runs. This is the
SAME bug class the plan already documented for the CHARACTER side ("Null access
.characterPxfContentMap at spawnPlayer", plan log ~line 119): a PXFResource whose
type-specific content map (characterPxfContentMap / stagePxfContentMap) is null.
Candidates (unverified — need a healthy channel):
  (a) the bare-name/prefix resolver picks a resource whose content was never
      queued+loaded, so its content map stays null;
  (b) builtin stage content uses a namespace other than `global::` (builtins live
      in assets/data/dat*.fra and may register under a different ns), so
      `global::st_battlefield.st_battlefield` is the wrong ref;
  (c) the queueResourcesFromMatchSettings path doesn't enqueue the stage's PXF
      content for our synthetic MatchSettings.

### What's needed (NOT trivial; needs a healthy channel)
1. Find a stage ref that actually loads its content map non-null. Enumerate the
   real loaded stage ids from the ResourceManager pool at runtime (the resolver's
   own registry-search path), or inspect dat*.fra manifests for a stage's true
   namespace::package.id, and pass that FULL id to `s` (skip the bare-name
   prefix-expand).
2. Re-run and confirm via the actual error.log that the
   `Null access .stagePxfContentMap` crash (md5 3537a487) is GONE.
Only after the stage loads can the match reach character logic, so the
elapsedFrames freeze oracle / physics telemetry / move-drive (#4,#6,#7) are all
downstream of this.

NOTE: do NOT confuse any of this with the converter freeze (removeAllEffects),
which is separately FIXED + source-verified. The match never even reached
character logic in these runs — it died at stage setup first.

## Move-drive (#4) status
playCState@6801 unverified on this channel (the `dis`/`fninfo` output got
fabricated mid-session). Re-verify `fninfo 6801` / `dis 6801` on a healthy
channel (md5-cross-check) before wiring an `m <stateId>` command. Building blocks
in main.rs: require_fn/require_field/find_type, add_int/add_string_const,
direct Opcode push + jump-offset patching (see the `s`/`q` handlers ~lines
994–1162). Get the Character via Match(field-6-or-_matches).characters[0]
(field 35).

## NEXT STEP (actionable, needs healthy channel) — fix stage resolution
Verified facts to build on:
- Char resolves+renders via `global::<id>.<id>` (buzzwole, sandbag both fine).
- Stage `global::st_battlefield.st_battlefield` resolves to a non-null resource
  but its stagePxfContentMap is null at Match.setupStage → crash (md5 3537a487).
- src/main.rs has NO `global::` literal; the resolver prefix-expands at runtime
  (tries custom::/public::/global:: and picks the first whose getPXFResource is
  non-null). For stages that picks a ref whose content map was never loaded.
- Builtin stages are in ENCRYPTED dat*.fra (can't parse ids from files).

To fix, on a healthy channel:
1. Disassemble the resolver block in connect_edit (the `s`-handler token-resolve
   path, ~lines 1044+) to see exactly how the stage token becomes a ref, and how
   the char path differs (char works, stage doesn't — compare the two).
2. Determine builtin stages' real content namespace: either
   (a) re-enable the registry-search path (poolHash iterate, currently bypassed
       via j_skipreg per match-launch memory) to find the live stage ref by id, or
   (b) find the engine getter that loads a stage's PXF content (analogous to
       getCharacterContent@18292 / getStageContent@18297 from memory) and call it
       so stagePxfContentMap populates before setupStage, or
   (c) pass a full `namespace::package.id` for a builtin stage and confirm via
       error.log that 3537a487 is gone.
3. Cross-check with buzzwole (known-good) as the control each time.
Only after the stage loads does the match reach character logic; then
elapsedFrames (Match field 75) gives a real freeze oracle, unblocking #4/#6/#7.
