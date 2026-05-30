# Headless UGC load — per-angle attempts with bytecode-level evidence

Goal: spawn converted sandbag into a match via a path we control, WITHOUT Steam.
Crash being solved: `Null access .characterPxfContentMap` at
`pxf.core.Match.spawnPlayer` (Match.hx:1423) because `getPXFResource(sandbag)`
returns null — sandbag's resource is never in `poolHash`.

All findexes/ops below are sha-verified disassembly of hlboot-sdl.dat (HLB v4).

## Angle 1 — inject `UgcUtil.loadUgc@17796` at READY (MainMenu ctor)
- Opcode injected: `Call0 { fun: RefFun(17796) }` prepended in inject_ready_flag.
- Runtime: no error change; resolver still falls to `global::sandbag.sandbag`;
  spawn still null. 15s AND 50s post-READY waits both fail.
- Why: loadUgc → loadInLocalUgc → loadInSubscribedUgc → _checkIfAllDirectoriesLoaded.
  The subscribed path + the async per-file load never complete headless.

## Angle 2 — inject `UgcUtil.loadInLocalUgc@17842` (local-only, skips subscribed/Steam)
- Opcode injected: `Call0 { fun: RefFun(17842) }`.
- loadInLocalUgc@17842 disasm (sha d375f22c): sets ugcEverythingExpectedQueued,
  calls findLocalUgc@17836 (scans getCwd()+"custom"), done. No Steam, no guards.
- Runtime: identical failure. So the gate is NOT the subscribed/Steam path; the
  LOCAL async file-read pipeline (addDirToLoadQueue@17837 → load@1845 with
  success/error callbacks) never fires its completion headless.

## Angle 3 — Steam interpose shim v1/v2 (DYLD_INSERT_LIBRARIES)
- Built x86_64 shim interposing SteamAPI_IsSteamRunning→true,
  SteamInternal_SteamAPI_Init→0, GetHSteamUser/Pipe→1, RestartAppIfNecessary→false.
- hl has hardened runtime (codesign flags=0x10000) → macOS STRIPS
  DYLD_INSERT_LIBRARIES. Defeated by running an ad-hoc re-signed copy `hl_shim`
  (codesign --remove-signature + `codesign -f -s -` → flags=0x2 adhoc). Verified
  shim FIRED (SHIM_FIRED:1, "[steamshim] ... forced OK" in engine log).
- Runtime: engine STILL logs `[API loaded no]`; spawn still null. Faking
  individual Steam fns doesn't populate the engine's Steam context.

## Angle 4 — Steam shim v4: call the REAL SteamAPI_InitFlat from a ctor
- Shim ctor sets SteamAppId=1420350, calls real `SteamAPI_InitFlat(err)`.
- Runtime evidence (sha-verified engine log):
    [steamshim] InitFlat=0 running=1 err=''
  => **Steam DOES fully initialize in our headless process (InitFlat returns
  0=OK, IsSteamRunning=1).** YET the engine's own line still says `[API loaded
  no]`. So `[API loaded no]` is the ENGINE's independent init verdict
  (SteamInternal_SetMinidumpSteamID path), set BEFORE/separately from our shim,
  and the UGC gate keys off the engine's own boolean — not off whether Steam
  actually works. Fighting the SDK layer is the wrong layer.

## Angle 5 — manual `getBytes → Resource.fromBytes → finishLoading → addResource`
- Injected a synchronous load using findexes I read as getBytes/fromBytes/
  finishLoading. **BUT fninfo proved those findexes were WRONG** (corrupted
  reads): 64=hl.types.ArrayBase.pushDyn, 2520=pxf.core.Match.addTimer — NOT
  loaders. So this injection was a no-op/garbage; unchanged crash is meaningless.
- CORRECT primitives now identified (sha-verified from importManifest@18228 +
  addDirToLoadQueue@17837 disasm):
    * `pxf.io.$Resource.__constructor__@17827(res, path, privatePath, encTypeRef)`
      — stores path metadata only, delegates to AbstractResource ctor@1869.
    * `pxf.io.AbstractResource.load@1845(res, successCb:t395, errorCb:t395)` —
      t395 are CALLBACK closures → load is CALLBACK/async-shaped, op11
      `CallThis proto#0` is the real loader. This is the likely headless-stall point.
    * `pxf.io.$ResourceManager.addResource@18230(res)` → poolHash (final registrar).
    * importManifest@18228 builds Resources from a manifest array and addResource's
      them, but is gated by `_lockManifestImport` (field 45): op2
      `JFalse _lockManifestImport → op9`; when true it logs "disabled" + returns.

## KEY UNRESOLVED FACT (blocks the clean fix)
`addResource@18230` has exactly TWO callers: UgcUtil._onFileLoaded@17838 (async,
stalls headless) and importManifest@18228 (gated/"disabled"). **Builtins
(assets/data/dat*.fra) spawn fine, so they reach poolHash via one of these —
almost certainly importManifest at boot with _lockManifestImport=false at that
moment, then re-locked.** If so, the surgical fix is: at READY, set
`_lockManifestImport=false` and call importManifest@18228 with a 1-entry manifest
descriptor for sandbag (dynobj {path, privatePath, type:"character",
encryptionType}). The blocker on executing this cleanly has been TOOL-CHANNEL
unreliability corrupting the multi-KB disassembly reads needed to nail the
descriptor field layout + confirm load@1845's sync/async nature.

## TWO CONCRETE PATHS (both authorized, pick to continue)
A. **Bytecode manual register** (no Steam): inject New Resource@17827 + set fields
   + load@1845 (with no-op callbacks) OR set _lockManifestImport=false + call
   importManifest@18228 with a synthetic sandbag descriptor, at READY. Requires
   reliable disasm of the descriptor shape (channel permitting). Highest control.
B. **Reversible hlboot swap + real Steam launch** (gets genuine [API loaded yes]
   so the real UGC pipeline runs, custom + workshop both): back up
   hlboot-sdl.dat.orig, write our patched bytecode to hlboot-sdl.dat, launch via
   Steam, restore on exit. Writes a Steam file reversibly — the user re-authorized
   patching a local copy / opting to replace. Highest reliability, lowest novelty.

Recommendation: B is the reliable spawn TODAY; A is the pure-no-Steam path the
user prefers but needs a healthy channel for the descriptor RE.
