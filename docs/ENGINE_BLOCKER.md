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
  patcher (`tools/fraymakers-harness/src/main.rs`). Stopped bytecode surgery to
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
