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
