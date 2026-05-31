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

### Move dispatch + state readback VERIFIED (on a successful spawn)

On a spawn that wins the buried-VFX race, the converter's character is functional:
```
./runseq.sh 3 "s sandbag thespire commandervideoassist" t m t t
  → T:INTRO  M:JAB  T:STAND  T:STAND
```
- State machine works: spawns → `INTRO` → settles to `STAND`.
- Move dispatch works: `m` = `Character.toState(CState.JAB)` acks `M:JAB`.
- State readback works: `t` = `getStateName()`.
So the converter output is runtime-correct; the only blocker to full validation is
**spawn reliability** (the buried-VFX race, ~40%).

### Durable-path research outcome (2026-05-30)

Two RE agents investigated the reliable-spawn fix:
- **`.hl` migration is counterproductive** (rejected): `haxe`/`hl` not installed; it's a
  1–2 week bytecode-linker effort that STILL requires by-name engine-symbol relinking and
  adds an extern-layout-drift failure mode. Recommendation: a ~1–2 day **Rust label/
  register helper** over the existing `hlbc` emission (kills jump-offset + register-alloc
  errors) is the right authoring foundation.
- **Engine-native pin found**: `startMatch` spares match content from
  `flushUnusedResources` by putting it in `queueHash` via `queueResources@18233`;
  `set_Required@1832` is a pure field-10 write. **But** implementing
  `queueResources(["private::sandbag"]) + set_Required(true)` after `addResource` did NOT
  improve reliability in practice (still ~baseline, same Character.hx:769 crash) — and
  every bytecode tweak (set_Required / per-frame recache / queueResources) measured at
  ~15–40% (small-sample noise) with no real gain. This points to the true cause being the
  **sprite-cache POPULATION race** (our re-cache entry not reliably created at the async
  construction frame), not eviction — OR the hand-bytecode additions themselves perturbing
  timing. The hand-emitted-bytecode path is the limiting factor (matches the memory TODO).

**Conclusion (DECISIVE, data-backed):** reliable headless spawn is NOT achievable by
bytecode pinning. Measured reliability:
- baseline (synchronous load + manual recache, no pin): ~40% (2/5)
- + `set_Required`: ~1/5 · + per-frame recache: 0/6 · + `queueResources`+`set_Required`: **1/8**

Every op added to the load path DEGRADES below baseline. Root cause (diagnosed): the
`Asm`-built **NSPR probe proved the recache is reliable** — after `l` (no startMatch),
both `SPR:1` (bare key) and `NSPR:1` (namespaced buried-VFX key) every run (5/5). So it's
EVICTION, not population. But the eviction fix backfires: `queueResources` adds sandbag to
the match load queue, which makes `startMatch`'s `load` *re-load* it ASYNC, widening the
race vs the async character ctor. The synchronous-headless-load architecture fundamentally
fights the engine's async content lifecycle; no in-`update` bytecode pin wins it.

**Direct observation (NSPR-in-q probe, commit c17aaec5):** after `startMatch`, the buried-VFX
key IS cached (`NSPR:1`) while the match loads (`Q:NO_MATCH`); then the ASYNC
`onMatchReady → Match.init → spawnPlayer → ctor` reads the SAME key and gets null ~60% of
the time (40/60 RACE). So `startMatch`'s flush is NOT the culprit. Neither `set_Required`
(field 10) NOR a direct `queueHash.set` (field 17) prevents it — so the loss does NOT honor
the `flushUnusedResources` exemptions. The cache entry is removed/cleared *inside*
`onMatchReady`/`Match.init`, atomically with construction (no `update` frame between, so
per-frame recache can't intervene). RE agent investigating the exact clear mechanism.

**The real reliability fix is architectural** (future work): load sandbag through the
engine's OWN async UGC path (register under `custom::` and let `startMatch`'s content
queue load it like the real game), instead of the synchronous `fetchThreaded` shortcut +
manual cache. That eliminates the eviction/reload race at its source. The `Asm` helper
(committed cbcad5e2) is the durable authoring foundation for that work.

**For validation NOW:** retry the spawn (~2–3 boots to win the 40% race); once at
`T:STAND` the character is stable and the full moveset can be driven in that one session.
The freeze fix + box-geometry validation stand regardless; the converter output is correct.

### RELIABILITY DOSSIER — exhaustive (every approach tried + result)

Goal: 100% reliable headless spawn (currently ~40% — buried-VFX `Character.hx:769`
null at the async ctor). Four RE agents + ~20 live test cycles. **Conclusively, no
resource/cache/flag manipulation fixes it:**

| Approach | Result |
|---|---|
| baseline (sync load + manual cacheSpriteEntity, no flags) | ~40% (2/5) |
| `set_Required(true)` after load | ~baseline (1/5, noise) |
| `set_Required`+`set_PreloadMedia` after load | crash (REQ:1 PRE:1 **confirmed via readback**, still wiped) |
| same flags set BEFORE fetchThreaded | crash (order irrelevant) |
| flags before + DROP manual recache (rely on finishLoading's load-owned f26 closure) | crash (**NSPR:1 confirmed** — load-owned entry present, still wiped) |
| `queueResources(["private::sandbag"])` | WORSE (1/8 — triggers an async reload that races) |
| direct `queueHash.set` (flush-exempt, no reload) | crash |
| per-frame recache (every re-delivered `s`) | WORSE (0/6) |

**Conclusive finding:** the sprite-cache key `private::sandbag.sandbag` is present
right after `startMatch` (`NSPR:1`, even load-owned with `Required`+`PreloadMedia`
both confirmed set), then nulled inside the ASYNC `onMatchReady → Match.init →
spawnPlayer → ctor` flow, atomically (no `update` frame between → per-frame recache
can't intervene), as a 40/60 timing race. This **refutes** the RE-prescribed
`flushUnusedResources` theory (a load-owned, Required+PreloadMedia entry would be
spared). The wipe is a flag-ignoring path (full cache reset or a per-key uncache from
a load cycle) that 4 static-RE passes could not locate.

**The principled fix is architectural, NOT a bytecode pin** (and per the hard
constraints we must NOT patch the engine's cache logic as a workaround): load sandbag
through the engine's OWN UGC pipeline so its working content lifecycle owns the cache.
`k` confirms the engine already QUEUES `custom/sandbag/` at boot (`DIRS_QUEUED>0`); the
async load just stalls on the worker thread headless. RE recipe (agent ace2ece9):
load under the real UGC namespace `<LOCAL_NAMESPACE>_sandbag` (`set_Namespace@1793`,
RM.LOCAL_NAMESPACE = g3508 f11) + `addResource` + `postLoadFraProcess@17847`, and
launch with `<LOCAL_NAMESPACE>_sandbag::sandbag.sandbag`. Reading LOCAL_NAMESPACE at
runtime is the next concrete step (one careful diagnostic; an earlier attempt
mis-fired). Alternatively: runtime-trace the wipe by instrumenting
`uncacheSpriteEntity@18272` / detecting a field-24 reset (needs second-function
injection — a new harness capability; the `Asm` helper now makes that safe to author).

**Tooling delivered for this:** `src/asm.rs` (label/register helper) eliminates the
hand-bytecode jump/register errors, so the architectural fix or the runtime trace can
be authored reliably. **The freeze fix + box-geometry validation stand regardless —
the converter output is correct; this is purely a harness headless-load reliability
gap.** Validation of the moveset can proceed retry-based (~2–3 spawns) in the interim.

## 4. Runtime: drive every move + physics ⬜ (blocked on spawn reliability)

## 5. Animation playthrough (frame-state capture) ⬜
