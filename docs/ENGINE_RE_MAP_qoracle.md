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

## THE REAL CURRENT BLOCKER (verified via error.log): WRONG STAGE ID
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

Root cause of the null stage: the probe sent stage id `battlefield`, which the
bare-name resolver prefix-expands to `global::battlefield.battlefield` (the
LAUNCHED ack confirms it resolved to *a* resource) — but the REAL builtin stage
content ids carry an `st_` prefix. From the install's
assets/data/stages.json (read directly):
  "stages": ["st_battlefield","st_grid","st_thespire","st_finaldestination_2",
             "st_pixelandia","st_smallbattlefield","st_warning"], hub "st_hub".
So `battlefield` resolved to a non-stage / wrong resource whose
stagePxfContentMap is null → setupStage crash. (Memory said `thespire` "worked"
before; the correct id is `st_thespire`. Re-verify which form the resolver needs.)

### Fix (trivial, no engine-injection needed): use a valid stage id
Use `st_battlefield` (or `st_thespire`). freeze_probe.sh now defaults to
`st_battlefield` (override via FRAY_STAGE). Once the stage is valid, setupStage
succeeds, the match runs, and Match.elapsedFrames (field 75) advances — giving a
REAL freeze oracle: sample elapsedFrames twice; fixed .fra advances, a frozen
build stalls. That also unblocks physics telemetry (#7, Match.players[0] char
pos/vel) and move-drive (#4, play-state lever on Match.characters[0]).

VERIFICATION STATUS: the corrected-stage re-run was attempted but the tool
channel degraded to fabricating/empty output mid-run, so its result is NOT yet
trusted. Re-run `FRAY_STAGE=st_battlefield ./freeze_probe.sh stagefix` on a
healthy channel and confirm via the actual error.log file: the
`Null access .stagePxfContentMap` crash (md5 3537a487) should be GONE.

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
