# P1 / P2 status — SSF2 functional parity + modder features

Honest status of every item in the P1/P2 mandate. P0 (spawns + animates + no
crash) is done for 45/45 characters; this tracks the higher bars.

## P1 — SSF2 functional parity

| Item | Status | Notes |
|---|---|---|
| Parity-measurement harness | **DONE** | `DUMP_PARITY` raw-SSF2 dump + `tools/parity_check.py` (static SSF2-source-vs-output diff). Achieved the parity goal without `.hl`. |
| Per-move reference specs | **DONE** (auto) | The `DUMP_PARITY` JSON IS the per-move SSF2 source-of-truth; `parity_check` maps SSF2→FM fields and diffs. |
| Verify mario + sandbag, fix divergences | **DONE** | 45/45 chars **hitbox-STAT parity** (damage/angle/baseKnockback/knockbackGrowth/hit-freeze). Fixes: jab-split inheritance, `weightKB`→baseKnockback, present-keys mapping. |
| Branch-arm stack-threading | **DONE** | Both BranchCmp + Branch arms seed branch bodies with the predecessor stack; recovered `/* ? */` lost exprs. |
| 5 empty-shell characters | **DONE** (bonus) | The frame-data check exposed fox/bomberman/donkeykong/pit/luffy as empty shells; sprite-label fix recovered all 5 to full movesets (verified animating in-engine). |
| Haxe→`.hl` spike | **SPIKED → deferred** | Toolchain works (haxe 4.3.7, `haxe -hl`, hlbc reads it). Blocker: a trivial Haxe = 322 functions, so "loading" into the engine = a full cross-module linker (index remap + dedup). A focused linker project — `tools/peptide/hl_spike/README.md`. |
| Physics-stat tuning | **DEFERRED (empirical)** | The SSF2-derived movement stats (gravity/fall/walk/jump/weight) ARE mapped + scaled; friction/accel/ECB are hand-tuned constants. Getting the FM scale FACTORS right needs in-engine SSF2-vs-converted comparison (the dummy/`.hl` measurement path), not a static fix. |
| Frame-data: active-frame range | **PARTIAL** | Per-char hitbox COVERAGE done (caught the 5 shells). Exact active-frame-range comparison vs SSF2×2 is fiddly (30→60fps doubling + split-anim slicing + keyframe length-encoding) — low signal-to-noise; the coverage check already catches the load-bearing "doesn't activate" case. |

## P2 — modder features

| Item | Status | Notes |
|---|---|---|
| Friendly command surface + help/gloss | **DONE** | spawn/move/state/physics/anim/query/load/keys/console/ping/exit + aliases. |
| `move <name>` (full moveset) | **DONE** | Generated CState jump table from `commands::MOVES`. |
| `loop <move> [count]` | **DONE** | Client-side repeated dispatch. |
| `snapshot` | **DONE** | state+physics+anim bundle (`Sequence`). |
| Recipe scripting | **DONE** | `recipe.sh` runs a `.recipe` file (commands + `#!char`/`#!gap`). |
| A/B comparison | **DONE** | `ab_compare.sh` golden behavioral-signature regression check. |
| Crash diagnostics | **DONE** | Bridge dumps the last ~16 events when the engine stream ends. |
| Animation scrubbing | **DONE** | `step` (pause+advance one frame via `playFrame`) + `play` (resume). |
| Stat hot-reload | **PARTIAL** | The `spawn` handler already re-loads a fresh `.fra` in-session, so "edit → re-export → `spawn` again" IS the hot-reload loop today. A mid-match in-place stat re-read is unresearched. |
| Dummy opponent + `hitresult` + `kill_threshold` | **DEFERRED (deep)** | The live-tuning loop's core. Needs a 2nd fighter (modify the proven `s` handler — opt-in to keep 45/45) + positioning (SetField body.x) + post-hit damage/knockback readback. A multi-step chain; the foundation (read player-N fields) is the proven physics pattern. |
| Save/restore engine state | **DEFERRED (deep)** | Needs the engine's match-serialization surface (unresearched). |
| State-machine introspection | **PARTIAL** | The `ANIM:` stream + crash-context history already give the transition trace; "legal transitions / why stuck" is engine-internal. |

## The honest bottom line

P0 corpus-wide (45/45) and **P1 hitbox-stat parity corpus-wide (45/45)** are DONE
and verified. The deepest P1/P2 items that remain — in-engine *behavioral*
measurement (dummy→hitresult), the `.hl` linker, physics multiplier tuning,
save/restore — all converge on ONE missing capability: **in-engine measurement of
emergent behavior** (what a move actually DOES to an opponent). That capability is
a focused next project (dummy + positioning + readback, OR the `.hl` linker for
richer logic). Everything built this session is the foundation for it.
