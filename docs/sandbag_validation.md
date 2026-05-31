# Sandbag — end-to-end validation

This doc records the harness commands + their expected outputs that confirm
**sandbag passes** the full converter → FrayTools → Fraymakers pipeline. It is
the template for validating the rest of the 47-character corpus.

Status legend: ✅ verified live · ⏳ in progress · ⬜ not started

---

## 0. Convert (clean) ✅

```
rm -rf characters/sandbag
cargo build --release
./target/release/ssf2_converter ~/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf
```

Expected: no `WARN`/`ERROR` lines. Final log lines:
- `Dropped 10 empty animation(s): egg, item_raise, jab2..4, ledge_*, taunt`
- `Generated: 28 attacks, 89 animations, 584 frame scripts`

`conversion_log.json` lists SSF2-source method call counts (`unknown`,
`ssf2_only`) — these describe *source* API usage, not emitted-code errors. The
generated `Script.hx` uses valid Fraymakers API (`match.createVfx`,
`AudioClip.play`).

---

## 1. FrayTools publish ✅

```
node tools/fraytools-harness/export-in-fraytools.js \
  --project "$PWD/characters/sandbag/sandbag.fraytools" \
  --fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools"
```

Expected stdout: absolute path to `characters/sandbag/build/sandbag.fra`.
"Publish All" also writes the engine custom dir. Verify identical bytes:

```
shasum -a 256 characters/sandbag/build/sandbag.fra \
  ~/Library/Application\ Support/Steam/steamapps/common/Fraymakers/custom/sandbag/sandbag.fra
```

Expected: matching SHA-256 (both 3,889,880 bytes as of 2026-05-30).

---

## 2. Box geometry vs SSF2 source ✅

FrayTools renders only **static** collision layers (hurt/item/body); hitboxes
are runtime script data and do not appear here (validated separately at
runtime, §4).

```
node tools/fraytools-harness/harness.js \
  --project "$PWD/characters/sandbag/sandbag.fraytools" \
  --entity entities/Sandbag.entity --animation <anim> --frame <n> \
  --out-json /tmp/box.json --port 9222
cargo run --release --bin compare_boxes -- \
  --ssf2 ~/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf \
  --char sandbag --json /tmp/box.json --tolerance 2.0
```

`compare_boxes` is **split-aware** (commit 1979730b): split sub-animations
(`strong_forward_attack`, `*_land`, `grab_hold`, …) are mapped back to their
source SSF2 anim + start-frame offset, so they verify instead of SKIP.

Sweep result (2026-05-30, fresh output) — **every box within 2px, 0 FAIL, 0 SKIP**:

| anim/frame | boxes | result |
|---|---|---|
| idle f0 | 3 | PASS (incl. rotated itembox 0.006px) |
| jab1 f2 | 4 | PASS |
| dash_attack f4 | 4 | PASS |
| tilt_forward f4 | 3 | PASS |
| tilt_up f4 | 3 | PASS |
| tilt_down f4 | 5 | PASS |
| strong_forward_attack f0/f6 | 4 | PASS (split → strong_forward +42) |
| strong_up_attack f4 | 5 | PASS |
| aerial_neutral f3 | 2 | PASS |
| aerial_forward f3 | 4 | PASS |
| aerial_up f3 | 4 | PASS |
| aerial_down f3 | 3 | PASS |
| grab f2 | 3 | PASS |
| special_neutral f6 | 3 | PASS |
| special_side f4 | 4 | PASS |

Note: the prior session's "itembox 3.7–7px FAIL" is **resolved** in current
output — the rotated-affine itembox anchor now lands at 0.002–0.006px.

---

## 3. Fraymakers boot + spawn ⏳ (freeze FIXED; intermittent buried-VFX race)

Boot + socket verified (`PONG`). `s` is self-bootstrapping (commit 9c0d3d05):
`./runseq.sh <gap_s> "s sandbag thespire commandervideoassist" t t t` — fires at
engine READY (event-driven), LAUNCHES with no prior `l`.

**Freeze FIXED (commit 0ab59180):** the standing animation was named `idle`;
Fraymakers' `CState.STAND` resolves the `stand` animation, so its absence hung
the engine main loop at character construction. Renaming `idle`→`stand`
(+`walk`/`fall`/etc. canonicalization pending) unfroze it. Sandbag now reaches
`Q:MATCH_LIVE` and `T:INTRO`→`T:STAND` (live state readback verified).

**KNOWN ISSUE — buried-VFX sprite-cache eviction race (~40% spawn success):**
`Character.__constructor__` (Character.hx:769) builds a "buried" VFX from
`statsProps.spriteContent` (`private::sandbag.sandbag`) via
`getSprite`→`pxfSpriteEntityCache`. `FraymakersMode.startMatch` calls
`ResourceManager.load`→`flushUnusedResources`, which evicts our headless-loaded
sprite-entity cache **before** the async `onMatchReady`→`spawnPlayer`→ctor reads
it → `Null access`. Confirmed: `l` alone gives `SPR:1` reliably (cache populated);
only match-start eviction makes it null. This is a **harness** headless-load
limitation, not a converter bug — the converter output is correct (verified when
the race is won: clean `T:STAND`). RE: `docs/` (createCharacter/buried-VFX agents).
Three bytecode mitigations (set_Required, per-frame recache) tried — all made it
worse; the hand-emitted-bytecode path has hit the ceiling flagged in
`memory/project_fraymakers-engine-internals.md` (migrate to pre-compiled `.hl`
before adding more). Current workaround: retry spawn until `T:STAND` (~2-3x).

### Deferred converter bug — IntervalTimer null callback (charge states)

Charge frame scripts emit `self.addTimer(8, -1, effects.get())` (e.g.
`strong_forward_charge`, `strong_up_charge`, `item_smash_charge`). The 3rd arg
is the timer *callback* but `effects.get()` is the effects **Array**, so when the
timer fires `pxf.util.IntervalTimer.process` (IntervalTimer.hx:73) null-derefs.
The abc_parser mis-resolved the SSF2 callback (likely `updateCharge`, which
exists in Script.hx) to the `effects` instance var. Only triggers when a smash is
charged — not on spawn/idle. Fix is in the ActionScript→hscript callback
resolution. DEFERRED until basic spawn/moveset validation is reliable.

## 4. Runtime: drive every move + physics ⬜

## 5. Animation playthrough (frame-state capture) ⬜
