# Handoff — autonomous sandbag conversion + harness work

## Where we are right now
Working autonomously to take **sandbag** end-to-end through the SSF2→Fraymakers
converter and validate it via both harnesses. Hit a **real, reproducible bug**
and was mid-diagnosis when interrupted.

**THE BUG (current focus):** Launching our converted characters in-engine
crashes at player spawn:
```
Exception: Null access .characterPxfContentMap
Called from pxf.core.Match.spawnPlayer (Match.hx:1423)
Called from pxf.core.Match.init (Match.hx:608)
Called from MatchController.onMatchReady (MatchController.hx:479)
Called from MatchController.~startMatch.2 (MatchController.hx:342)
```
The match LOADS (we get `<< LAUNCHED ...` ack) then throws at spawn. Reproduced
with a FRESHLY published .fra (not a stale-file artifact). Affects BOTH our
**sandbag AND our mario** → it's a converter/packaging OR harness-load-path bug,
NOT sandbag-specific.

**DECISIVE TEST RESULT (buzzwole):** `s buzzwole thespire ...` →
`<< LAUNCHED custom::buzzwole.buzzwole`, and **NO error.log** — buzzwole
SPAWNED CLEAN through the exact same harness `s` path that crashes our
sandbag+mario. STRONG signal that the bug is in **our .fra PACKAGING**, not the
harness load path. CAVEAT: read on a corrupting tool channel (stray tags, clipped
lines), and `custom::buzzwole.buzzwole` is an unexplained anomaly (buzzwole is
workshop content, not in custom/ — yet it resolved+loaded under custom::).
RE-VERIFY this result first thing on a healthy channel before fully trusting it.
If it holds: focus on why our .fra doesn't populate characterPxfContentMap that a
clean external .fra does — diff per-content-item declarations / the
cacheCharacterContent keying.

(superseded) Earlier framing of this test:
`./run.sh "s buzzwole thespire commandervideo.commandervideoassist"` —
background task `bvg5wq8ng`, log at `/tmp/claude-501/buzz_boot.log`.
- If buzzwole ALSO crashes with the same null → the bug is our **injected
  startMatch path not loading custom/workshop content before spawn** (builtin
  `commandervideo` spawned fine earlier via the same `s` command and rendered a
  real match). Fix would be harness-side (queue/await the character resource
  load before spawn).
- If buzzwole spawns clean → the bug is our **.fra packaging** (something the
  engine needs to populate characterPxfContentMap that our .fra lacks).
**RESUME BY READING `/tmp/claude-501/buzz_boot.log` + the engine error.log**
(`/Users/jimmy/Library/Application Support/Steam/steamapps/common/Fraymakers/error.log`).

NOTE: buzzwole is WORKSHOP content
(`steamapps/workshop/content/1420350/3631426073/buzzwole.fra`), not in custom/.
If the bare-name resolver can't find it (workshop may not be in the loaded
registry / poolHash), that itself is informative — may need to copy buzzwole
into `custom/buzzwole/` to test it the same way as sandbag.

## Key facts established this session (reliable)
- **Converter runs clean** on sandbag (exit 0, no WARN/ERROR): 28 attacks, 89
  animations, 584 frame scripts. The big "unknown" API counts (play ×220,
  createVfx ×129) are FALSE POSITIVES — they ARE rewritten in output (118
  AudioClip.play, 66 match.createVfx). Real residual unknowns are item/grab
  mechanics; defer until engine testing shows if they break.
- **Pipeline:** `ssf2_converter` CLI stops at FrayTools project files. The `.fra`
  is produced ONLY by FrayTools' own Publish, driven by
  `tools/fraytools-harness/export-in-fraytools.js` (CDP). No Rust .fra packer.
- **FrayTools is 0.4.0** (matches harness target). Earlier "0.5.0 blocker" claim
  was WRONG/fabricated.
- **.fra format:** 4-byte header (`00 10 XX XX`) + top-level JSON
  {audio,binary,entities,images,nineSlices,scripts,spritesheets,version} then a
  trailing binary blob region (bytesOffset/bytesLength refs). NOT zip/png.
- **Manifest diff (buzzwole vs sandbag):** both have resourceId,
  content[].type:"character", entity metadata.objectType:"CHARACTER". sandbag
  manifest has a 2nd content entry (characterAi). No obvious structural break —
  the bug is subtler than the manifest.
- **Custom-content install:** `custom/<id>/<id>.fra` + `meta.json`. meta.json is a
  FrayTools publish artifact (format: `{"steamId":"","description":"My Custom
  Fraymakers Content","version":"0.0.1","privacy":"public","changelog":"","title":"<id>"}`).
  Builtins live in `assets/data/dat*.fra`.

## Changes committed this session (branch `fraymakers-match-harness`)
1. `d9005a06` — **FIXED the FrayTools publish harness cold-launch race.** Root
   cause: `waitForCdp` only polled `/json/version` (HTTP up) but the renderer
   page target registers seconds later → `CDP()` threw "No inspectable targets".
   Added `waitForTarget()` polling `/json/list` for a real page/webview target.
   Cold launch now publishes sandbag.fra end-to-end (verified, exit 0).
   File: `tools/fraytools-harness/export-in-fraytools.js`.
2. Several `docs/autonomous-sandbag-plan.md` commits — the **stop condition** (6
   criteria for "100% done") + a detailed progress log. READ THIS DOC FIRST on
   resume; it has the full chronology including my corrected mistakes.

`tools/fraymakers-harness/run.sh` is the engine launcher — additive flow
(recreates _conn.dat + steam_appid.txt each run, never modifies hlboot-sdl.dat,
restores nothing because it adds nothing permanent). Usage:
`./run.sh "s <char> <stage> <assist>" [secs]`. Needs `dangerouslyDisableSandbox`.

## Process cautions (I made mistakes — don't repeat)
- Twice over-claimed success by misreading TRUNCATED/LAGGED tool output (claimed
  a boot worked when error.log actually had the null crash; fabricated a version
  blocker). **Always read results from files and quote the actual ack/log line
  before claiming success.**
- The tool channel was intermittently degrading (duplicated/clipped output).
  Mitigation that worked: route everything to files under `/tmp/claude-501/`,
  run long things via `run_in_background:true` + Monitor/Read, use unique end
  markers, verify suspect output with `shasum`/`wc -c`.

## Task list (TaskList tool)
Done: #1 stop-condition doc, #2 conversion+triage, #4 crash-vs-skip, #5 install.
In progress: #9 iterate-to-100% (BLOCKED on the spawn bug above).
Pending: #3 FrayTools harness pass, #6 RE engine input-dispatch for internal
control functions (the big "drive moves without keypresses" task), #7 physics
telemetry, #8 animation capture.

## Memory
Long-term notes in
`~/.claude/projects/-Users-jimmy--openclaw-workspace-main-ssf2-fraymakers-converter/memory/`
— esp. `project_fraymakers-match-launch.md` (harness internals, findexes) and
`project_fraymakers-engine-internals.md`.

## Immediate next steps on resume
1. Read `/tmp/claude-501/buzz_boot.log` + engine error.log → classify the bug
   (packaging vs harness-load-path) per the decisive test above.
2. If harness-load-path: inspect how the normal menu flow loads character
   resources before spawn (queueResourcesFromMatchSettings@... / ResourceManager
   load) and replicate in the injected `s` path before startMatch fires.
3. If packaging: diff how characterPxfContentMap gets populated — compare a
   builtin/buzzwole .fra's character content declaration vs ours at the
   per-content-item level (the cacheCharacterContent path keys off content type).
4. Re-confirm whether the tool channel is healthy before doing more binary RE.
