# Known issue: itembox rotated-anchor drift (~3.7px, scales with rotation)

## Severity: LOW (deferred)
Affects ONLY the itembox (item pickup/grab range), which is gameplay-noncritical.
Every hurtbox / hitbox / body box converts sub-pixel-accurate (FrayTools
validation: 0.000–0.001px). So this does not affect hit detection or hurt
detection — only where an item can be grabbed from.

## Evidence (sha-verified, FrayTools harness + compare_boxes)
- idle f0:           itembox rendered anchor (-14.316,-17.801) vs SSF2 expected
  (-10.600,-17.801) → drift 3.716px (X only; Y exact).
- aerial_neutral f3: itembox drift 7.002px.
- Drift SCALES WITH ROTATION → it's the rotation bake, not a constant offset.

## What the numbers say (computed, reproduced 4×)
Stored entity itembox (idle f0): x=-14.55 y=-51.9 rot=-22.79 pivotX=1.85
pivotY=21.85 scaleX=3.69 scaleY=21.85. The converter's bake
(entity_gen.rs:642-661) chooses the stored top-left so FrayTools' rendered
registration anchor `collision_box_anchor(stored,pivot,θ)` lands on the intended
hand point `intended_pivot_point(fb)`. For idle that intended hand = (-10.6,-17.8)
— which is EXACTLY the SSF2-expected value. But the value FrayTools actually
renders the anchor at is (-14.316,-17.801). So the bake's model of FrayTools'
anchor transform is slightly off for rotated boxes: it pins the *un-rotated*
pivot point, while FrayTools rotates the pivot about the box origin first.

## Likely fix direction (NOT yet applied — needs careful coordinate work)
The bake solves `stored = hand - collision_box_anchor(0,0,pivot,θ)`. The residual
== `collision_box_anchor(stored,pivot,θ) - intended_pivot_point(stored)` at the
SAME θ, i.e. the affine "offset" term used in the solve does not match the term
FrayTools applies. Re-derive `collision_box_anchor` against a FrayTools-rendered
ground-truth at 2-3 known θ (the harness can supply rendered anchors directly:
the `rendered_anchor` field in harness JSON IS FrayTools' truth), then make the
bake invert THAT exact function. The harness now makes this a tight loop:
  1. emit a candidate bake, 2. publish+harness one rotated itembox frame,
  3. read rendered_anchor, 4. adjust until rendered==SSF2-expected.
Do this on a healthy tool channel (binary/coordinate RE needs reliable reads).

## Why deferred now
- Low severity (above).
- The mandate's core engine work (drive moves, physics, animation capture) is
  blocked on a separate, higher-impact decision (headless UGC load — see
  ENGINE_BLOCKER.md), so itembox-px-polish is not on the critical path.
- This area has churned repeatedly in git history; a rushed fix under a degraded
  tool channel risks regressing the (perfect) hurtbox path.
