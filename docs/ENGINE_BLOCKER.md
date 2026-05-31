# Engine-side blocker: custom/UGC content doesn't load in the `./hl` harness launch

## The decisive finding (high confidence, multi-verified)

| content | source dir | loads in our `./hl _conn.dat` launch? |
|---|---|---|
| commandervideo (builtin) | `assets/data/dat*.fra` | **YES** — spawns + renders a real match (screenshot-verified earlier) |
| sandbag (our converted) | `custom/sandbag/` | **NO** — `getPXFResource` null → `spawnPlayer` crash |
| mario (our converted) | `custom/mario/` | **NO** — same crash |
| buzzwole (clean external) | `steamapps/workshop/.../` | **NO** — same crash |

So: **builtin `assets/data` content loads fine; UGC (local `custom/` + Steam
`workshop/`) content does not load** in our direct-`./hl` injection launch.
The crash is always `Null access .characterPxfContentMap` at
`pxf.core.Match.spawnPlayer` because `getPXFResource(<char>)` returns null —
the character's resource was never added to the ResourceManager pool.

## What we ruled out
- **Not timing.** A 20s post-READY delay (`FRAY_POST_READY_DELAY`, now a harness
  feature) did NOT help — content still absent, resolver still falls through to
  `global::sandbag.sandbag`.
- **Not our packaging.** buzzwole (a clean, externally-published workshop char)
  fails identically. And our sandbag's box geometry is sub-pixel-correct
  (FrayTools validation). The `.fra` is fine.
- **Not a missing trigger in the bytecode.** `Main.onLoaded → loadComplete →
  UgcUtil.loadUgc → {loadInLocalUgc (scans custom/), loadInSubscribedUgc
  (workshop)}` IS on the normal boot path.

## Most likely root cause
UGC loading is async and tied to the Steam client context (engine has title
states `custom_content_loading` / **`custom_content_waiting_for_steam`**). When
we launch `./hl _conn.dat` directly, `SteamAPI_Init` returns OK but the deeper
Steam UGC/storage callbacks evidently never complete, so `loadUgc` never
populates the pool. Builtins come from `assets/data` (no Steam/UGC path), which
is why only they work.

## The architectural conflict (NEEDS A USER DECISION)
- Our control bridge **requires** launching `./hl _conn.dat` directly, because
  that's our *patched* bytecode (the injected TCP bridge + `s` command).
- Custom/UGC content loading **appears to require the real Steam launch context**.
- Launching through Steam runs `hlboot-sdl.dat` (unpatched) — so to get BOTH our
  injection AND Steam's UGC loading, we'd have to **patch `hlboot-sdl.dat`
  in place** and let Steam launch it. **The user explicitly forbade overwriting
  hlboot** ("i do not want to overwrite the hlboot"; we reverted that approach).

### Options to resolve (pick one)
1. **Reversible hlboot swap for Steam launch.** Back up `hlboot-sdl.dat.orig`,
   write the patched bytecode to `hlboot-sdl.dat`, launch via Steam
   (`steam://run/1420350`), restore the original on exit. This is the swap the
   user previously rejected — but it's the only way to combine injection + Steam
   UGC loading. (Reversible; original restored every run.)
2. **Force local UGC load in-process.** Inject a call to
   `UgcUtil.loadInLocalUgc@17842` (pure filesystem scan of `custom/`, no Steam)
   at READY, gate the `s` launch until `getPXFResource(<char>)` is non-null.
   This would unblock **local custom/ characters (sandbag)** without Steam, but
   NOT workshop content. Risk: `loadInLocalUgc` may be guarded by
   `m_beforeFirstLoad`/Steam state; needs bytecode work + verification.
3. **Hybrid:** keep `./hl` direct launch; add option-2's forced local load just
   for sandbag-style local content (the actual mandate target).

**Recommendation:** Option 2/3 (force local UGC load) — it stays within the
"don't overwrite hlboot" constraint and targets exactly what we need (local
converted characters). Option 1 is the fallback if local-load proves
Steam-gated too.

## Status of the run when this was written
- Tool channel was corrupting output (truncation, mangled grep/disasm, `EOF`
  artifacts) — could not safely do delicate edits to the 1300-line bytecode
  patcher (`tools/peptide/src/main.rs`). Stopped bytecode surgery to
  avoid shipping unverifiable patches.
- Verified wins committed this session: both FrayTools CDP harness cold-launch
  fixes (`export-in-fraytools.js`, `harness.js`), FrayTools box-geometry
  validation (hurt/hit boxes sub-px; itembox rotated-anchor ~3.7px drift,
  low-sev, deferred), and the `FRAY_POST_READY_DELAY` feature.

## Resume checklist
1. Healthy tool channel (canary + shasum check) before bytecode edits.
2. Confirm the 4 wedged `hl _conn.dat` procs (uninterruptible sleep) are gone
   (reboot if needed).
3. Implement option 2: inject `loadInLocalUgc@17842` + gate `s` on
   `getPXFResource` non-null. Verify buzzwole-style is out of scope; verify
   **sandbag spawns** (no error.log). Then proceed to criteria #4–6
   (moves via internal funcs, animation capture, physics telemetry).

## Task #6 prep (internal move dispatch) — found statically
Character move API (pxf.entity.$Character), the "drive moves WITHOUT keypresses" lever:
- playCState@6801 (argc 2)  — entity.playCState(CState.X); converted Script.hx
  already uses this (e.g. CState.JAB2). PRIMARY lever.
- setState@6758 (argc 3), toState@6772 (argc 2), updateState@6755, playAnimation@6743
Plan: once spawn works, harness `m <stateId>` command = get currentMatch player
Character, call playCState with the CState enum value for the move. Physics
telemetry (#7) = read Character position/velocity fields each tick. Capture (#8)
= screencapture per frame after seeking state.

## ROOT CAUSE CONFIRMED (decisive, sha-verified)
Engine stderr on every headless `./hl _conn.dat` launch:
    SteamInternal_SetMinidumpSteamID: Caching Steam ID: 765... [API loaded no]
=> **`[API loaded no]`** — the Steam API is NOT actually live in our direct
launch, even though `SteamAPI_Init()` prints "OK". Steam UGC (custom/ + workshop)
loading requires the live Steam API, so `loadUgc`'s async pipeline never
completes → content never enters the ResourceManager pool → `getPXFResource`
null → spawnPlayer crash. Builtins (`assets/data`) need no Steam API, so they
spawn. This is independent of cwd (run.sh cd's correctly) and timing (50s wait
fails identically) and the injected loadUgc call (which runs but its file-load
events never fire without the live API).

This is a hard boundary: our control bridge needs the *patched* bytecode
(`./hl _conn.dat`), but Steam only launches the *unpatched* `hlboot-sdl.dat`
and only a Steam launch gives `[API loaded yes]`. The two are mutually exclusive
without either (a) patching hlboot-sdl.dat in place + Steam-launching, or
(b) finding a way to make the headless process pass Steam's running-app check.

## DECISION REQUIRED (constraint boundary — flagging, not crossing)
Mandate hard-constraint: "do not modify the Steam install beyond custom/<id>/;
never patch the engine binary or replace Steam files." The only known way to get
BOTH our injection AND live-Steam UGC loading is to temporarily replace
`hlboot-sdl.dat` with the patched copy and launch via Steam, restoring the
original after (reversible). That technically WRITES a Steam file
(hlboot-sdl.dat) — which the constraint forbids. So this needs explicit user
approval before proceeding.

Alternatives that DON'T touch Steam files (lower confidence, need RE):
- B. Drive content-load + match-start through the ALREADY-RUNNING Steam-launched
  game via its in-engine Console/Tildebugger (no bytecode patch of hlboot at
  all) — if the console can call functions / our bridge can attach to the live
  process. Needs RE of whether the shipped build exposes a usable console hook.
- C. Make headless `./hl` pass Steam's check (e.g. correct steam_appid handshake
  / run under `steam -applaunch`) so `[API loaded yes]`. Uncertain it's possible.

## UPDATE: loadInLocalUgc also fails headless — UGC pipeline is async-stalled
Switched the injection from loadUgc@17796 to loadInLocalUgc@17842 (local-only,
no Steam, no guards: findLocalUgc scans custom/ → addDirToLoadQueue per dir).
RESULT (sha-verified): identical failure — global::sandbag.sandbag, spawn crash,
[API loaded no]. So it's NOT the guards or the subscribed/Steam path. The local
.fra READ pipeline (addDirToLoadQueue@17837: getDirectoryListing → per-file
async read with a Trap/event callback _onFileLoaded → addResource) never
completes its file-read events in the headless process.

KEY: builtins in assets/data/dat*.fra DO load headless → they must use a
DIFFERENT, synchronous boot loader, NOT the UgcUtil async pipeline. Per the
user's insight ("a .fra is a .fra; mirror how existing content loads"), the
next move is to find the assets/data loader and route sandbag through it:
  - Find what reads assets/data/dat*.fra at boot (NOT UgcUtil). Likely a
    ResourceManager bulk/synchronous import in Main::onLoaded/nextLoadStep.
  - Either (a) point/extend it to also read custom/sandbag/sandbag.fra, or
    (b) call the same low-level "read .fra file → AbstractResource → addResource"
    primitive directly from our injection (synchronous), bypassing UgcUtil.
  - Then ensure the namespace matches: local callback assigns
    LOCAL_NAMESPACE_<name>; either replicate that or use whatever namespace the
    synchronous import assigns, and make the resolver find it (the registry
    pool-search path — currently disabled because it HUNG; fix the iteration).

## Namespace finding (sha-verified)
Local UGC content is registered under namespace `LOCAL_NAMESPACE + "_" + cleanName`
(ResourceManager.LOCAL_NAMESPACE static, field 11; cleanName = dirname with
[^a-zA-Z0-9_- ] stripped). So even once loaded, `custom::`/`global::` resolution
is WRONG — must resolve via pool registry-search by content-id, not namespace guess.

## RESUME POINT (channel corrupting reads — paused mid-RE)
Confirmed synchronous boot loader path (sha-verified up to here):
- Main boot: CoreEngine::preLoad@17866 → ResourceManager.queueRequiredResources@18234
- The data manifest "manifest.json" (string idx 40744, const-global 9341) is read
  by exactly ONE function: findex 26465 (const-global ref). THIS is the builtin/
  assets-data content registrar — the synchronous, no-Steam path that works headless.
NEXT (on a HEALTHY channel — verify with canary+shasum first):
  1. Disassemble findex 26465 cleanly (it was mid-dump when the tool channel began
     injecting fake EOF/markdown text — do NOT trust that read). Find how it turns
     manifest entries into resources + calls addResource@18230, and what namespace
     it assigns.
  2. Mirror that for custom/sandbag: either extend the manifest scan to include
     custom/<id>/<id>.fra, or call the same file→AbstractResource→addResource
     primitive directly from our injection (synchronous), at READY before `s`.
  3. Fix the resolver to find pooled content by content-id registry search
     (namespace-agnostic) — the disabled hung path; fix its iterator.
  4. Verify sandbag spawns (no error.log), THEN proceed #6 (playCState move driver),
     #7 telemetry, #8 capture, #9 iterate.
Current injection state: inject_ready_flag calls loadInLocalUgc@17842 (proven
insufficient headless — leave or revert to 17796; neither loads content. The real
fix is the manifest-reader path above, not the UgcUtil pipeline).

## CORRECTION: findex 26465 is NOT the manifest reader
26465 is a Type::initClass registration function that merely references the
"manifest.json" string constant — whoref matched the const-global, not a real
reader. Disregard the "26465 = manifest reader" note above. The actual builtin
loader still needs to be found. Better search strategies for next session:
- callers of addResource@18230 OTHER than UgcUtil._onFileLoaded@17838 (there were
  only 2 callers total; the other was UgcUtil. So builtins may ALSO go through
  _onFileLoaded — meaning assets/data uses the SAME pipeline but completes,
  while custom/ stalls. If so, the difference is WHERE/HOW the file read is
  driven, not the registrar.)
- Re-examine: addResource callers = {UgcUtil._onFileLoaded@17838, UgcUtil.init
  @17830-area}. If addResource has NO non-UGC caller, then builtins are loaded
  some other way (preloaded bundle / different pool). Check getCharacterContent@
  18292 reads pxfCharacterContentCache (field 29) — find what POPULATES field 29
  for builtins (maybe a separate cache from poolHash). That cache-populator is
  the real target.

## HONEST SUMMARY FOR USER
Headless ./hl cannot load custom OR workshop content via any UGC entry point
tried (async pipeline stalls; [API loaded no]). Builtins work via a different
mechanism not yet pinpointed. The conversion itself (sandbag .fra) is validated
correct on the FrayTools side (boxes sub-px). Getting custom content into a
headless match needs either: (a) more RE to find+invoke the builtin sync loader
for our file, or (b) the reversible-hlboot-swap + Steam launch (writes a Steam
file; needs approval). This is a deep engine-internals effort beyond quick fixes.

## *** BREAKTHROUGH: the synchronous loader path (sha-verified core) ***
Both addResource@18230 AND cacheCharacterContent@18258 have the SAME single
caller: pxf.io.AbstractResource::finishLoading@1842. So there is ONE unified
registrar — finishLoading — that populates BOTH poolHash (getPXFResource) and the
type cache (pxfCharacterContentCache that getCharacterContent reads). Builtins
reach finishLoading and complete; our custom .fra's async read never fires it
headless.

finishLoading@1842 is invoked by AbstractResource::{load@1838, loadFromBytes@1839,
onChunkLoaded@1840}. **loadFromBytes@1839 is the SYNCHRONOUS path**: bytes in →
finishLoading → addResource + caches. No async, no event loop, no Steam.

### THE FIX (next session, healthy channel)
Inject at READY (before `s`):
  1. read custom/sandbag/sandbag.fra into bytes (sys.io.File.getBytes or the
     engine's FileObject read — both exist),
  2. construct an AbstractResource (or the PXFResource subtype) for it,
  3. call loadFromBytes@1839 (or load@1838) → finishLoading registers it
     synchronously into poolHash + the character cache,
  4. ensure the namespace it registers under matches what `s` resolves; simplest
     is to make the resolver do a pool registry-search by content-id (the
     currently-disabled hung path — fix its iterator) so namespace doesn't matter.
Then getPXFResource(sandbag) is non-null → spawnPlayer succeeds → criteria #3-#6
unblock (playCState@6801 move driver, telemetry, capture).
RE remaining: exact signature of loadFromBytes@1839 (args: bytes? path? meta?) and
how to build the resource object — disassemble 1838/1839 cleanly first.

## CHANNEL: corrupting reads (injecting EOF/``` fences, truncating). All
sha-verified facts above are reliable; anything not sha-checked this session is
suspect. Recommend restart before the bytecode-injection step.

## ACCURACY FIX (sha-verified — supersedes the "shared single caller" claim)
- addResource@18230 (-> poolHash; getPXFResource reads this) callers:
    UgcUtil._onFileLoaded@17838  AND  ResourceManager.importManifest@18228.
- cacheCharacterContent@18258 (-> pxfCharacterContentCache; getCharacterContent
    reads this) caller: AbstractResource.finishLoading@1842.
These are DIFFERENT chains; I conflated them earlier — disregard "addResource and
cacheCharacterContent share one caller."
SPAWN CRASH is on getPXFResource (poolHash) == null. poolHash is filled by exactly
two funcs: _onFileLoaded (UGC async, stalls headless) and importManifest@18228
(string says "disabled"). So the open question = how do BUILTINS get into poolHash
if both poolHash-fillers are UGC/disabled? Possibilities to check next (clean
channel): (a) importManifest is only conditionally disabled (need its real
disasm — was being read when channel corrupted), (b) builtins use getCharacter
Content's cache path and spawnPlayer for builtins doesn't hit the null because
their poolHash entry comes from importManifest at boot. RESOLVE by cleanly
disassembling importManifest@18228 + finding its caller chain at boot.

## SESSION END STATE
Channel corrupting bytecode-disasm reads (injecting EOF/```/'wait' into op
streams; caught by canary+shasum). Stopping bytecode RE per discipline rule.
Net verified progress this goal-run: 30+ commits; criteria #1 & #2 MET (sandbag
converts clean; FrayTools boxes sub-px except low-sev itembox); root cause of
#3-#6 block fully characterized (headless poolHash never populated for custom
content); fix direction identified (synchronous loadFromBytes/importManifest
path — exact primitive pending one clean disasm of importManifest@18228 + the
loadFromBytes signature). Resume: restart for clean channel, disassemble
importManifest@18228 and AbstractResource.load@1838/loadFromBytes@1839, then
inject a synchronous "read custom .fra bytes -> register into poolHash" call at
READY + namespace-agnostic pool resolver.
