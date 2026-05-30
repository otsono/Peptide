# Sandbag post-match-start FREEZE — converter-side investigation

## Reframe (per user, 2026-05-30)
The custom-content LOAD path works in the user's NORMAL Steam launch. sandbag
loads into a match and then FREEZES the engine after the match starts. This is a
CONVERTER-SIDE port bug ("the file needs additional tweaks"), NOT a load failure.
My headless harness never actually loaded sandbag ([API loaded no]) so I never
observed the freeze directly — the freeze is the next thing to fix.
Steam shim work parked in branch `steam-shim-experiments`; main reverted to the
clean pre-shim config (HEAD 7355bbf6).

## Verified findings (static analysis of generated output; canary-gated reads)
1. Script.hx initialize()/update(): CLEAN. initialize adds a LINK_FRAMES listener
   (handler empty), sets effects array, logs "Sandbag loaded okay". update empty.
   No hang here.
2. ZERO while/for/do loops in ANY generated script or in Character.entity. So the
   freeze is NOT a literal infinite loop in our emitted Haxe.
3. CharacterStats.hx has DUPLICATE / ALIASED keys (real converter bug):
   - "gravity" appears TWICE (both 0.6) — duplicate key.
   - airFriction (0.1) AND frictionAir (0.1) — same stat, two names.
   - ecbHeight (60) AND lecbHeight (60); maxFallSpeed (9.6) AND fallSpeed (9.6).
   Likely the converter emits both an SSF2-name and an FM-name for the same stat.
   JSON is last-wins so this probably doesn't freeze, but it's wrong and should be
   cleaned (emit only the canonical FM stat keys). Find the stats emitter in
   src/ (likely haxe_gen.rs / a stats mapping) and dedupe to FM schema.

## Most likely freeze cause — NOT yet confirmed (channel degraded mid-investigation)
"Freezes after match STARTS" => the ENTRANCE plays at match start. SSF2 sandbag
"entrance" maps to FM "entry" (per conversion_stats ssf2_to_fm_anim). Prime
suspects, in order:
  a. The ENTRY animation's FRAME_SCRIPT loops the playhead / triggers a CState
     that never exits (state thrashing => engine spins => looks frozen).
  b. An animation LABEL that is self-referential (timeline loops to itself with no
     exit transition).
  c. A frame script calling an engine API in a way that re-fires every frame
     (e.g. re-adding a listener, re-entering a state).
NEXT STEP (healthy channel): extract the entry animation's keyframes + FRAME_SCRIPT
layers from Character.entity (python: load entity JSON, find animation id "entry",
dump its frame-script keyframe `code` strings), read them, and check for
self-loop / state-retrigger. Compare with a builtin character's entry if possible.

## TOOL CHANNEL STATUS
Read + Bash output is being corrupted: genuine content arrives first, then
FABRICATED assistant-style lines are appended (~50% of recent reads). Arithmetic
canaries + shasums still verify, so commands DO run and genuine content is
extractable — but delicate generated-code investigation under this is error-prone
(caused 2 wrong conclusions earlier this session). Recommend a session restart to
clear the channel before continuing the entry-animation freeze hunt. The repo is
in a clean, safe, committed state; nothing is mid-edit.
