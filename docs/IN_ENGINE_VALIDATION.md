# In-engine validation — sandbag plays, NO freeze (stop condition MET)

Date: 2026-05-30

## Result
The converter-side freeze fix is validated in a live Fraymakers match.

```
$ fray_patch --fraymakers "<steam>/Fraymakers" --workspace ./workspace --launch
[fray_patch] applying patch: inject_ready_flag @ loadUgc
[fray_patch] READY socket on /tmp/frayremote.sock

$ frayremote ping                       -> READY
$ frayremote s sandbag battlefield none -> match started: sandbag vs (cpu) on battlefield
$ frayremote f (x2, 4s apart)           -> frame 47 -> frame 196      (advancing)
$ frayremote f (x3, 5s apart)           -> 372 -> 521 -> 670          (monotonic, ~30fps)
$ frayremote q players                  -> [{"id":0,"char":"sandbag","x":-120,"y":420,"state":"idle","damage":0}]
engine alive throughout.
```

VERDICT: **PLAYING_NO_FREEZE, SUSTAINED.** Before the loop-termination fix,
sandbag froze the engine immediately after match start (the removeAllEffects
infinite loop fired every frame via its GameObjectEvent.LINK_FRAMES listener).
Now the match clock advances steadily and the engine stays responsive.

## Path (no Steam, no shim)
- `fray_patch` patches a WORKSPACE COPY of hlboot-sdl.dat (Steam install
  untouched), injects a READY flag, launches the engine.
- `frayremote` drives it over /tmp/frayremote.sock (`s`/`q`/`f`/`ping`).
- The engine reads the converted character from the install's
  `custom/sandbag/sandbag.fra` (md5-verified to match the freeze-fixed build).

## Root cause + fix (recap)
counter_of_cond in src/decompiler.rs only matched loop counters that were
renamed to GetLex("i"); the common SSF2 form keeps the counter as an
un-renamed Local(n) that merely *renders* as "i". Those loops (sandbag's
removeAllEffects + 9 other chars) escaped the termination guard. Fix: compare
the RENDERED lhs of `i < ….length` against i/j/k/l. Verified: 120 counter-loops
across 45 chars, 0 non-terminating (was 9). Commit 8aa3f1c2.

## Remaining (next phase — needs frayremote extended)
frayremote currently exposes only s/q/f/ping; it has NO move-drive command.
To finish plan criteria #4–#6:
- #4 Drive moves via INTERNAL engine functions (playCState@6801, NOT keypresses)
  — add a `frayremote move <player> <state|input>` command that calls the
  engine's play-state function over the socket.
- #5 Physics/state telemetry — extend `q` (velocity, ecb, hitboxes per frame).
- #6 Animation capture — per-frame framebuffer grab keyed to match clock.
Then iterate sandbag to 100% (all moves, physics parity, no warnings).
