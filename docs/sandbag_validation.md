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

## 3. Fraymakers boot + spawn ⏳

(pending — see below)

## 4. Runtime: drive every move + physics ⬜

## 5. Animation playthrough (frame-state capture) ⬜
