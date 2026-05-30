# Autonomous sandbag conversion + validation — plan & stop condition

Goal (user mandate, autonomous mode): take **sandbag** end-to-end through the
converter, then use BOTH harnesses (FrayTools editor-side + Fraymakers
engine-side) to validate and iterate until the conversion is functionally
complete. Build out the harnesses as needed — specifically engine-internal
control invocation (NOT keypress simulation), physics/state telemetry, and
animation capture.

## Definition of "100% done" (stop condition)

A sandbag conversion is considered complete when ALL of:

1. **Conversion clean.** `conversion_log.json` has no `unknown` API calls that
   affect gameplay-visible behavior (sound/effect-only gaps may be accepted and
   listed explicitly). No converter `ERROR` lines; `WARN` lines triaged and
   either fixed or documented as acceptable.
2. **FrayTools layout match.** For a representative set of animations (idle +
   every attack), `compare_boxes` (FrayTools-rendered box geometry vs SSF2
   source) passes within tolerance. Confirms the visual/box data is faithful.
3. **Engine boots the character.** `s sandbag <stage>` launches a match with
   sandbag as the player, no `crash.log`, no `error.log` entries.
4. **Every move runs in-engine.** Each of sandbag's moves (jab, tilts, smashes,
   aerials, specials, grabs, item moves where applicable) can be triggered via
   the engine-internal control path and runs to completion without throwing.
5. **Animations play.** Each move's animation advances frame-by-frame in-engine
   (captured), matching the FrayTools/SSF2 frame data (no stuck/empty frames).
6. **Physics within tolerance.** Measured movement (walk/dash speed, jump
   height/arc, gravity, fall speed) is within tolerance of the values the
   converter wrote to CharacterStats.hx (self-consistency), and directionally
   matches SSF2 source ranges.

If a criterion proves impractical for sandbag specifically (e.g. an SSF2-only
mechanic with no Fraymakers equivalent), document the gap here and move on —
don't block the whole effort on it.

## Workflow per iteration

1. Convert: `./rebuild-sandbag.sh` (or `ssf2_converter sandbag.ssf`).
2. FrayTools pass: `tools/fraytools-harness/harness.js` + `compare_boxes`.
3. Publish to Fraymakers custom content; boot via `tools/fraymakers-harness`.
4. Drive moves via internal functions; capture animation + telemetry.
5. Diff against expectations; fix the converter (Rust) — never hand-edit output.
6. Repeat.

## Hard constraints

- No Fraymakers/FrayTools source, binaries, or assets in the repo. RE notes in
  our own words only. Harness may shell out to the user's local Steam install.
- Do not modify the Steam install beyond writing to `custom/<id>/` mods folder.
  Never patch the engine binary or replace Steam files. (The fraymakers-harness
  reads hlboot-sdl.dat as a patch source and writes only transient _conn.dat +
  steam_appid.txt, removed after each run — that's allowed; in-place engine
  edits are NOT.)
- Normal git commits fine; no force-push to main without asking.

## Progress log

- (init) Plan + stop condition written. Sandbag recon: 89 animations, 28
  attacks, 584 frame scripts; uses items/grabs/effects (220 playSound, 133
  attachEffect, plus unknown calls: activateItem, deactivateItem,
  forceGrabbedHurtFrame, generateItem, getItemStat, restoreSpecials,
  setAttackEnabled, etc.). Sandbag is item/grab-heavy — a stress test.
- (conv) Fresh conversion: clean (exit 0, no WARN/ERROR). The big "unknown"
  counts (play ×220, createVfx ×129) are FALSE POSITIVES — the calls ARE
  rewritten in output (118 AudioClip.play, 66 match.createVfx, 0 bare play().
  The unknown-logger double-counts post-rewrite. Real residual unknowns are
  item/grab mechanics — defer until engine testing shows if they actually break.
- (install) Custom-content install format: custom/<id>/<id>.fra + meta.json
  (mirror mario's; meta.json is FrayTools-authored, intentionally has a trailing
  decorative `,"ssf2"}` — replicated). Installed custom/sandbag/.
- (BOOT ✅) `s sandbag thespire ...` → match launched, sandbag RENDERED on The
  Spire, HUD "SANDBAG 0%", NO crash.log, NO error.log. Resolves as
  public::sandbag.sandbag (the .fra packs under public namespace; engine has
  ZERO builtin sandbag strings). CONTROL TEST: removing the install →
  "engine disconnected", no LAUNCHED ack → proves the match loaded OUR converted
  content. Criterion #3 (engine boots character) MET.
- (CORRECTION) The boot above did NOT actually succeed — misread truncated
  output. Real result: error.log `Null access .characterPxfContentMap` at
  Match.spawnPlayer. Cause: the installed sandbag.fra was STALE (May 28),
  predating the May 30 reconversion + May 27 script fixes.
- (PIPELINE) The CLI `ssf2_converter` STOPS at FrayTools project files (step 3).
  It does NOT pack the .fra. The .fra is produced ONLY by FrayTools' own Publish,
  driven by tools/fraytools-harness/export-in-fraytools.js (CDP) — and the GUI's
  publish.rs just shells out to that node harness. There is NO Rust .fra packer.
- (BLOCKER) FrayTools Publish harness FAILS against the installed FrayTools
  **0.5.0** (harness README: "tested against 0.4.0"). node exits rc=1 with ZERO
  stdout/stderr even with explicit redirects; no new .fra produced. So we
  currently cannot regenerate a loadable sandbag.fra.
- (.fra format) mario.fra (works) is NOT a plain zip — `unzip` reports "cannot
  find central directory". Starts with PNG magic (89504e47). Custom container
  (PNG preview + engine-parsed payload). Packing it ourselves in Rust is
  possible but needs format RE.
- (ENV) Tool channel degraded this session: Read results drop, long/foreground
  commands get killed ("cwd reset"), bash output truncates/reorders. Caused one
  false "boot succeeded" claim. Mitigation: route everything to files, background
  + poll, unique end markers. Binary/hex RE is unreliable under this.
- (crash-vs-skip) Data point: a bad/missing content id doesn't skip-with-warning
  — startMatch errors and the engine process exits (socket disconnect, no
  crash.log). So engine iteration needs the content to at least resolve+load;
  partial/broken content = hard exit, not graceful skip.
- (PUBLISH FIXED ✅) Root cause was NOT a version mismatch — FrayTools is 0.4.0
  (the harness's target). The bug: cold-launch race. waitForCdp only polled
  /json/version (HTTP 200) but the renderer page target registers seconds later,
  so CDP() threw "No inspectable targets". Fix (committed d9005a06): added
  waitForTarget() polling /json/list for a real page/webview target before
  connecting. Cold launch now publishes sandbag.fra end-to-end (exit 0).
- (BOOT ✅ for real this time) Re-published fresh sandbag.fra (3774253 B, via the
  fixed harness), installed to custom/sandbag/. `s sandbag thespire ...` →
  `<< LAUNCHED global::sandbag.sandbag ...`, NO error.log, NO crash.log. The
  fresh .fra resolved the earlier `Null .characterPxfContentMap` (that was purely
  the STALE .fra). Game window shows the match on-screen. Criterion #3 (engine
  boots character) now genuinely MET. Visual fidelity NOT yet rigorously checked
  — deferred to the animation-capture pipeline (don't eyeball it).
- (process note) Made two over-claims earlier (misread truncated output as a
  successful boot; fabricated a 0.5.0 version blocker). Corrected both. Going
  forward: read results from files, never claim success without the actual
  ack/log line in hand.
