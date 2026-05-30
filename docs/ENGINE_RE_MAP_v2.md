# Engine RE map v2 — re-derived by NAME (build = hlboot-sdl.dat @ 2026-05-30)

Supersedes findex citations in older docs (many were stale; e.g. old "playCState@6801"
is wrong — 6801 here is `noDirectionalInfluenceBehaviorCallback`). **Always re-verify a
findex with `fninfo` before use.** Source: 4-agent static RE workflow (high confidence).

## A/B. Load path — how a character reaches the engine (the headless blocker)

Two SEPARATE registries, both filled SYNCHRONOUSLY (no thread/Steam) by distinct primitives:

| Registry | RM field | Read by | Filled by |
|---|---|---|---|
| `poolHash` (+`pool`) | 13 (12) | `getPXFResource` | `addResource@18230` — `poolHash.set(fqid, resource)` |
| `pxfCharacterContentCache` | 29 | `getCharacterContent@18292` | `cacheCharacterContent@18258` ← `finishLoading@1842` |

- **`addResource@18230(resource)`** — synchronous `poolHash.set(getFullyQualifiedResourceId, res)`
  + `pool.push`. Also fills `resourcesHash`/`resources` *only if* `resource.get_Loaded()` true.
- **`importManifest@18228(manifest)`** — loops manifest `.data[]`, builds a `pxf.io.Resource`
  per entry (ctor@17827: id, path, ResourceType via `fromString@22266`) and calls `addResource`.
  **Gated by static `_lockManifestImport` (RM field 45)**: `Main.preLaunch@17755` calls
  `lockManifestImport@18229` as its FIRST op, permanently setting the lock → at runtime
  importManifest early-returns ("...has been disabled..."). No unlock fn exists.
- **Builtins** load fully synchronously at boot: `Config.configLoaded@17732` decodes an embedded
  garbled-JSON manifest → `Config.manifest` (field 9) → `importManifest@18228`; then
  `CoreEngine.configLoaded@17865` re-imports + `queueRequiredResources@18234` + `load@18242`.
  Manifest paths live INSIDE the decoded JSON (no `assets/data` string in bytecode).
- **Custom `.fra`** reaches `addResource` ONLY via `UgcUtil._onFileLoaded@17838` (op0=addResource),
  an async file-load completion callback. Headless w/o live Steam UGC, it never fires → poolHash
  stays empty → `getPXFResource` null → `spawnPlayer` crash. (Confirmed: `[API loaded no]`.)

### C. Synchronous content-publish primitive
- AbstractResource in this build has **NO `loadFromBytes`/`onChunkLoaded`** (stale names).
- `finishLoading@1842(this)` — synchronous; iterates the resource's decoded PXFResource content
  maps and dispatches each into RM caches: `cacheBinaryResource@18254`, `cacheCharacterContent@18258`,
  `cacheProjectileContent`, … `cacheMenuContent`. All are plain `StringMap.set` into static caches.
  **Does NOT call addResource.** The async gate is in `load@1845 → fetch` (proto#0), not here.
- Resource object: ctor `__constructor__@1869(this, id→f0, path→f3, t241 ResType→f4)`;
  namespace f1 defaults `"private"`; ResType inferred from extension (pxf→PXF) if code null.
- Content struct for a character = t1975 (id f8, name f10, resource:PXFResource f12, script f13,
  scriptId f14, statsProps f15, type f16).
- **Sync recipe (if we can parse .fra→PXFResource):** ctor → `set_DataAsPxf@1826(pxfres)` →
  `finishLoading@1842` (fills content caches) → `addResource@18230` (fills poolHash).

### REMAINING GAP (blocker #1)
The synchronous `.fra bytes → PXFResource` parser (populates `characterPxfContentMap` etc.) is
NOT yet identified — AbstractResource is abstract; `fetch`/proto#0 (the byte source) lives in an
unidentified concrete subclass, and may be event-loop/thread gated. Need: (a) disasm `spawnPlayer`
in THIS build to see exactly what it dereferences after `getPXFResource` (does it lazy-load?), and
(b) find the concrete resource subclass + its synchronous parse entry, OR confirm
addResource-then-trigger-`load@18242` completes on the MAIN event loop (vs the dead UGC worker).

## D. Character move-dispatch + telemetry API (FULLY MAPPED — build now)

Live entity: **`pxf.entity.Character` (type 783)**.

**Move dispatch:** `toState@4497(char, cstateInt, animName:String)` — primary (60+ callers);
`setState@4493(char, cstateInt)`. Move id is a **CState Int** read at runtime exactly as the
engine does: `GetGlobal(3946)` (`pxf.entity.$CState` statics) → `Field(N)`. Field indices:
STAND=7, FALL=24, JAB=68, DASH_ATTACK=69, TILT_FORWARD=70, TILT_UP=71, TILT_DOWN=72,
STRONG_FORWARD=75, STRONG_UP=78, STRONG_DOWN=81, AERIAL_NEUTRAL=82, AERIAL_FORWARD=83,
AERIAL_BACK=84, AERIAL_UP=85, AERIAL_DOWN=86, SPECIAL_NEUTRAL=87, SPECIAL_SIDE=88,
SPECIAL_UP=89, SPECIAL_DOWN=90, GRAB=96. (Reference call pattern: `fallFromLedge@4276`.)

**Live Character path:** `MatchController` statics `currentMatch` = field 6 → `Match` (type 634)
`.characters` = field 35 (ArrayObj of Character; `characters[0]` = player 0). (`.players`=f34.)

**Telemetry fields:**
- state: Character f17 `m_state` (Int CState id; name via `getStateName@4491`→`CState.constToString@19270`), f53 `stateName` (String), f18 `m_stateGroup`.
- position: Character f32 `body` (Body t571) → x=f18, y=f19 (Float).
- velocity: Character f38 `physics` → x_velocity=f24, y_velocity=f25, currentVelocityX=f26, currentVelocityY=f27.
- damage/%: Character f35 `damage` (t767) → _damage=f20, _effectiveDamage=f21, _maxDamage=f22.
- animation: Character f37 `animation` → currentAnimation=f23 (String), currentAnimationIndex=f24, currentFrame=f25, totalFrames=f26.
- facing: Character f34 `transformation` (t573) → scaleX=f21; `get_isFacingLeft@682` = scaleX<=0.

Key findexes: toState=4497, setState=4493, getStateName=4491, CState.constToString=19270,
isFacingLeft=682, fallFromLedge=4276 (ref). CState statics global=3946, Match type=634,
Character type=783.
