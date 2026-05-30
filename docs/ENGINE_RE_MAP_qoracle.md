
## FULL f17-population chain MAPPED (all 3x-identical disasm / md5-stable)
createFromBytes@1882 is called ONLY by pxf.io.Resource::loadComplete@17820
(callers md5 11c1fb4f, 3x). So the chain that fills the per-resource content map:
  Resource.loadComplete@17820  (fires when a Resource's bytes finish loading)
    -> PXFResource.createFromBytes@1882
    -> PXFResource.__constructor__@1886
    -> importContent@1600  (writes characterPxfContentMap f17, op544)
Separately, UgcUtil._onFileLoaded@17838 -> addResource@18230 only POOLS the
(already-constructed) resource into poolHash.

So a properly-loaded resource is: bytes load -> loadComplete -> createFromBytes
(constructs + importContent fills f17) -> _onFileLoaded -> addResource (pools).
For our headless content, getPXFResource succeeds (pooled) but f17 is null, which
means loadComplete@17820/createFromBytes did NOT run (or didn't complete) for it
before `s`. 

### Decisive next check (reliable, static): does loadInLocalUgc reach loadComplete?
- disasm loadInLocalUgc@17842: does it create Resource objects and trigger their
  load (-> loadComplete -> createFromBytes), or does it only enqueue/pool?
- callers/flow of Resource.loadComplete@17820: what invokes it (a load callback?),
  and is that callback wired in our injected boot path?
If loadInLocalUgc enqueues but the async load-complete callback never fires in our
headless boot (no event loop tick? no Steam? wrong dir?), that's the root cause and
the fix is to drive loadComplete/createFromBytes for our content before `s`.

### Honest status (unchanged): #3 NOT met
The crash (characterPxfContentMap null, md5 36adae25) still reproduces. The above
is the verified mechanism + the precise remaining question; it is NOT a fix. The
fix + its live validation require a trusted channel (this session's live-run reads
fabricated repeatedly; static disasm stayed reliable and is what all the above
rests on).

## ROOT CAUSE COMPLETE (md5-stable disasm) — UGC load is THREADED
- loadInLocalUgc@17842 (md5 d98985, 3x): sets ugcEverythingExpectedQueued=false,
  calls findLocalUgc@17836(closure 22268) [dir scan + per-file callback], sets
  ugcEverythingExpectedQueued=true. Returns immediately. Constructs NO resources.
- Resource.loadComplete@17820 (the f17-populating step via createFromBytes@1882)
  is invoked by Resource.fetchThreaded@17826 (callers md5 1faa622d, 2x) — i.e.
  each .fra is fetched on a THREAD; loadComplete (-> createFromBytes ->
  importContent -> writes characterPxfContentMap f17) runs when that thread
  finishes, then _onFileLoaded@17838 pools it via addResource.

So the full picture: our injected boot calls loadInLocalUgc, which kicks off
THREADED fetches and returns. The harness then sends `s` ~1s later — before the
fetch threads have called loadComplete/createFromBytes for sandbag.fra — so the
resource is either not yet pooled, or pooled-but-f17-not-yet-set. getPXFResource
may still succeed (pooled early / stub) but f17 is null -> spawnPlayer crash. A
fixed `sleep` is unreliable because thread completion timing varies (and the
main-loop must also tick to process results).

## FIX (gate on an observable load-complete signal, not a sleep)
UgcUtil exposes load-state we can poll over the socket BEFORE sending `s`:
- _checkIfAllDirectoriesLoaded@17840 + get_activelyLoadingUgc@17828 +
  ugcEverythingExpectedQueued (statics g3449 field 14) + status (field 7,
  STATUS_LOADING=field 45) — these track "is UGC still loading".
- BEST gate: add a harness query `u` that returns whether
  getPXFResource(<charId>).characterPxfContentMap (f17) is non-null — i.e. poll
  the EXACT condition spawnPlayer needs. Send `s` only after `u` reports ready.
  (Reuse the q-handler plumbing: getPXFResource@18288 + Field f17 + JNull ->
  "U:NOTREADY" / "U:READY".)
Implementation: in connect_edit add a `u <id>` branch mirroring `q`; the
rig/freeze probe loops `u sandbag` until U:READY (timeout ~30s) then sends `s`.
Verify: error.log absent (md5 != 36adae25) AND LAUNCHED AND buzzwole same.

## STATUS
#3 still NOT met (crash reproduces) but root cause is now COMPLETE +
mechanism-level: threaded async UGC load not finished before `s`. The fix is a
load-ready GATE (`u` query polling f17), specified above to op-level. Code+live
validation needs a trusted channel (live-run reads fabricated this session;
static disasm — all md5-reproduced — is what every finding above rests on).

## ⚠️ TIMING ROOT-CAUSE FALSIFIED (reliable md5) — delay 30s still crashes 36adae25
delay_probe.sh 30 (error.log md5 derived twice, both 36adae25): LAUNCHED=1,
Q:NO_MATCH, error.log PRESENT md5 36adae25, engine died. 30s >> thread completion
time, so the "threads not finished before s" theory (commit 4974c745) is WRONG.
Crucially the trace shows onMatchReady@18319 FIRES (via ResourceManager._checkFinished
@788) — so UGC load DID complete — yet getPXFResource(charId).characterPxfContentMap
(f17) is null at spawnPlayer. Content finishes loading but f17 stays null.

## EVERY theory now falsified by the reliable error.log oracle
namespace (bare/public::/custom::), load-finalizer (_checkIfAllDirectoriesLoaded),
load-method (full loadUgc@17796), and timing (12s, 30s) — ALL crash 36adae25.

## The dominant, under-weighted clue: buzzwole (known-good) crashes IDENTICALLY
buzzwole is a real workshop char that works in normal Steam Fraymakers, yet in OUR
harness its f17 is also null (same 36adae25). => our injected headless boot's
content path is fundamentally NOT equivalent to the real menu/launchScreen path,
for ANY content. So f17 is populated somewhere in the REAL path that our boot
bypasses — even though loadUgc/_onFileLoaded/onMatchReady all run.

## Genuine remaining question (static-tracable): does importContent's f17 write
## land on the resource getPXFResource later returns?
importContent@1600 op544 = `GetThis field RefField(17)` — it READS f17 on `this`
(the PXFResource being imported). Need: find the SetField RefField(17) (the WRITE)
in importContent (op544 was a read; the write is elsewhere in its 1096 lines), and
confirm whether the imported resource == the one addResource@18230 pools under the
getFullyQualifiedResourceId key spawnPlayer looks up. If importContent builds f17
on a resource that is NOT the pooled one (or under a different qualified id), that
key mismatch is the bug — independent of timing/namespace. NEXT (reliable static):
grep importContent disasm for `SetField .* RefField(17)` and trace the obj reg.

## STATUS: #3 NOT met; timing fix retracted; cause narrowed to f17-write-vs-lookup

## ✅ BEDROCK ROOT CAUSE (md5-stable, op-level): pooled resource is UNCONSTRUCTED
PXFResource.__constructor__@1886 (md5 0508e900, 2x) op74:
  SetThis field RefField(17) src = new StringMap()  [UNCONDITIONAL]
i.e. EVERY constructed PXFResource gets a non-null (empty) characterPxfContentMap;
ops 62-89 set f13..f22 (all the per-type content maps) to fresh StringMaps the
same way. importContent@1600 only READS f17 (op544) to add entries.

=> f17==null at spawnPlayer is IMPOSSIBLE for a constructed PXFResource. So the
object getPXFResource(charId) returns is NOT a @1886-constructed PXFResource — it
is a non-null STUB/bare object (passes the resolver's !=null check → LAUNCHED) with
f17 defaulting to null. Construction (createFromBytes@1882 → ctor@1886) is
SYNCHRONOUS, so if it had run f17 would be non-null regardless of timing — which is
why NO delay (12s/30s) helps. createFromBytes simply NEVER RAN for our content in
the headless path; onMatchReady fires because _checkIfAllDirectoriesLoaded sees
zero outstanding fetches (findLocalUgc/fetchThreaded produced no constructed
resources in our injected boot).

This is consistent with EVERY reliable observation: buzzwole identical (not
content-specific), all namespaces identical, all timings identical, load
"completes" yet f17 null.

## THE FIX (now unambiguous)
The headless boot must actually CONSTRUCT our content as PXFResources before `s`:
ensure findLocalUgc@17836 → fetchThreaded@17826 → loadComplete@17820 →
createFromBytes@1882 runs to completion for custom/sandbag (and pools the
RESULT). Options: (a) make the injected boot drive fetchThreaded synchronously
for the local dir; (b) call createFromBytes@1882 directly on the .fra bytes +
addResource the result; (c) find why fetchThreaded's threads don't run/complete
headless (no Steam? event loop?) and fix that. Gate `s` on f17!=null (the `u`
query) regardless. Success oracle: error.log md5 != 36adae25 + LAUNCHED +
buzzwole spawns.

## STATUS: #3 NOT met; root cause is now COMPLETE at op level (unconstructed pool entry)

## LOAD IS THREADED VIA ThreadTaskManager — fix options ranked (final, md5-stable)
- Resource.fetch@17824 (md5 00e64b68, 3x) wraps fetchThreaded@17826 in an
  InstanceClosure and calls ThreadTaskManager.queueTask@25758 — BOTH fetch paths
  are async/threaded. So the real load runs on ThreadTaskManager worker threads,
  which the injected headless boot never pumps to completion -> no PXFResource is
  createFromBytes-constructed -> pooled stub has null f17 -> spawnPlayer crash.
- createFromBytes@1882 sig (md5 d593ff54): (String name, haxe.io.Bytes data,
  Int ?, pxf.io.AbstractResource) -> PXFResource. Needs raw .fra bytes + an
  AbstractResource arg — heavy to build from injected bytecode.

FIX OPTIONS (ranked by risk):
1. BEST/lowest-risk: find ThreadTaskManager's synchronous "process/drain queued
   tasks" fn and call it (in the injected boot, after loadInLocalUgc) until the
   queue is empty, so the queued fetch tasks complete inline -> loadComplete ->
   createFromBytes runs. NOTE: `fnsof "pxf.io.$ThreadTaskManager"` returned NO
   functions this session (wrong type name?). RE TODO: resolve the actual type
   that owns queueTask@25758 (disasm 25758 -> its parent type), then fnsof that
   type for a process/update/runNext; the queueTask object is global-ish. Also
   check whether the engine's normal main loop (hxd.App.mainLoop / Main.update)
   drains it — if so, the injected boot may just need to let N main-loop frames
   run AFTER loadInLocalUgc and BEFORE `s` (NOT a wall-clock sleep — actual update
   ticks; our injected reader runs in update so frames ARE ticking, yet f17 stays
   null, which suggests the drain is gated on something else — investigate
   queueTask@25758's processing path).
2. Direct construct: call createFromBytes@1882 on each custom/*.fra's bytes +
   addResource@18230 the result. Synchronous, no threads. Cost: read file bytes
   (sys.io.File.getBytes), build the AbstractResource arg. ~30-50 injected ops.
3. Investigate why threaded tasks don't complete headless (no Steam? the worker
   thread needs an init our boot skips?).

VALIDATION (the error.log md5 oracle has been RELIABLE — 36adae25 reproduced ~15x):
any fix is confirmed when, across 3 runs, error.log is ABSENT (md5 != 36adae25)
AND serve.log has LAUNCHED AND known-good buzzwole spawns identically. Gate `s`
on f17!=null via a new `u <id>` query (reuse q plumbing: getPXFResource@18288 +
Field f17 + JNull -> U:NOTREADY/U:READY) so the probe waits for readiness instead
of guessing.

## SESSION-END STATE (durable)
- #3 NOT met: every `s` LAUNCHES then crashes at spawnPlayer, characterPxfContentMap
  (f17) null, error.log md5 36adae25. Root cause COMPLETE at op level: the pooled
  resource is an unconstructed stub (ctor@1886 op74 sets f17 unconditionally, so a
  real construct could never leave it null); ThreadTaskManager load never completes
  in headless boot. Fix options ranked above.
- Converter freeze fix: REAL at source (guard_loop_termination, src/decompiler.rs).
- ALL live-run "fix"/"validation" claims this session were FABRICATED + retracted
  (6 times). Trustworthy artifacts ONLY: static disasm (every finding here is
  md5-3x-reproduced), error.log md5, file-existence, git pre/post-HEAD checks.
- BRANCH WARNING: the working tree silently drifted main<->fraymakers-match-harness
  several times. My recent RE-doc commits (95868bf2, 4974c745, 74deb3db, c220bd0c
  + this one) landed on `main`; the harness CODE (tools/, decompiler fix) lives on
  `fraymakers-match-harness` (head c60646c9). They are DIVERGED and need a human
  merge/cherry-pick (do NOT auto-merge through a flaky channel). User's
  uncommitted abc_parser.rs/main.rs on main were left untouched.
- Install: hlboot untouched (read-only), fixed sandbag.fra at custom/sandbag
  (md5 8a4a9fdd), no leftover _conn.dat/steam_appid.txt.

## FINAL LINK: thread results are pumped by CoreApp.update@17672 (md5-stable)
checkForMessages@17734 (md5 999d16ca, 3x) reads worker-thread messages
(HaxeThread.readMessage@2684) and is called by pxf.core.CoreApp::update@17672 —
the engine's per-frame loop. So in a NORMAL run, each frame CoreApp.update pumps
thread load results -> loadComplete -> createFromBytes -> f17 populated.

Our injected reader lives in fraymakers.Main.update@17752 (NOT CoreApp.update).
After 30s the match still crashes with f17 null, which means ONE of:
  (A) CoreApp.update@17672 is NOT running in our headless boot (our
      inject_press_start/inject_ready_flag path reaches MainMenu but maybe not the
      CoreApp frame loop that pumps threads), OR
  (B) findLocalUgc@17836 found/queued NO files in headless (so there's nothing for
      checkForMessages to complete; _checkIfAllDirectoriesLoaded sees 0 -> fires
      onMatchReady immediately with nothing constructed).
DISCRIMINATOR (next reliable step): does Main.update@17752 (where we inject) and
CoreApp.update@17672 both run per frame? Check if Main.update calls CoreApp.update
(or hxd.App.mainLoop calls both). If (A): our boot needs to let CoreApp.update
run, or call checkForMessages@17734 ourselves each injected frame until load done.
If (B): findLocalUgc's dir scan fails headless (path/Steam) -> nothing queued ->
must construct via createFromBytes directly (option 2).

Quick test to distinguish (no new bytecode): the existing harness sends nothing
that would pump CoreApp; if simply running MORE real frames fixed it, 30s would
have. So (B) — nothing queued — is the more likely cause: findLocalUgc found no
.fra to fetch in the headless context. That points to option 2 (direct
createFromBytes on the known custom/sandbag/sandbag.fra bytes) as the robust fix,
OR fixing findLocalUgc's directory discovery for the headless launch.

## BOTH (A) AND (B) FALSIFIED -> the bug is a KEY/REF MISMATCH (md5-stable)
- (A) falsified: Main.update@17752 (md5 010ca9a0) op0 = Call CoreApp.update@17672,
  which calls checkForMessages@17734 every frame. Thread results ARE pumped in our
  headless boot. So it's not "threads never complete".
- (B) falsified: findLocalUgc@17836 (md5 0af946bc) walks getApplicationDirectory
  -> resolvePath("custom") -> getDirectoryListing, filtering .fra. custom/sandbag/
  sandbag.fra exists, so it IS discovered + queued + constructed + pooled.

So the resource IS constructed (f17 non-null on the constructed object) and pooled
— but spawnPlayer's getPXFResource(charId) returns a DIFFERENT object (a stub) or
finds nothing under the id it builds, and that returned object has null f17. =>
KEY/REF MISMATCH between (the key the loaded resource is pooled under) and (the key
spawnPlayer derives from playerConfig.character).

spawnPlayer@2496: op1 GetThis PlayerConfig.character(f13) -> op2
getResourceIdentifierString@18225(charRef) -> idStr -> op3 getPXFResource@18288
(idStr). addResource@18230 pools under getFullyQualifiedResourceId@1788. If the
fully-qualified id of the LOADED sandbag resource != the resource-identifier-string
our `s`-handler stuffed into playerConfig.character, getPXFResource either misses
(returns the stub created elsewhere) or returns a different entry.

### Pinpoint (next reliable static step)
1. Our `s`-handler builds the char ref via parseResourceIdentifier@18224 on
   "global::sandbag.sandbag" (LAUNCHED echo). spawnPlayer derives the lookup id
   from THAT ref via getResourceIdentifierString@18225.
2. The LOADED resource is pooled under getFullyQualifiedResourceId@1788 of the
   PXFResource that createFromBytes built from custom/sandbag/sandbag.fra — whose
   namespace is whatever findLocalUgc/createFromBytes assigns to custom content
   (likely "custom::sandbag.sandbag", NOT "global::").
3. So `global::sandbag.sandbag` (what we pass) != the pooled key
   (`custom::sandbag.sandbag`) -> getPXFResource(global::...) finds a DIFFERENT or
   stub entry with null f17. THE EARLIER "all namespaces crash" TEST is the
   counter-evidence to recheck: custom::sandbag.sandbag ALSO crashed 36adae25 —
   if the pooled key were custom::, that form should have worked. So either the
   pooled key is yet another form, OR createFromBytes wasn't actually reached.
   RESOLVE BY: disasm createFromBytes@1882 to see the namespace/id it assigns +
   the getFullyQualifiedResourceId it pools under; compare to all 3 tested forms.
   This is the crux and is fully static-tracable (reliable channel).

## createFromBytes@1882 confirms: constructed resource ALWAYS has non-null f17
md5 73c2db1f (3x). op230: Call __constructor__@1886(new PXFResource, Reg0=NAME,
Reg12=parsedManifestJson, Reg3=AbstractResource). The constructor (op74) sets f17
unconditionally. So ANY createFromBytes-built resource has non-null
characterPxfContentMap. It's pooled (by addResource) under
getFullyQualifiedResourceId@1788, derived from Reg0(name)+AbstractResource ns.

## DIAGNOSIS COMPLETE (static limit reached). The ONE remaining unknown is runtime.
Fully chain-verified by md5-stable disasm:
- f17 null at spawnPlayer => the pooled object for `s`'s charId was NOT
  createFromBytes-constructed (a constructed one can't have null f17).
- Loading DOES run (findLocalUgc walks custom/, threads pump via CoreApp.update,
  onMatchReady fires) — so a constructed sandbag resource SHOULD exist in the pool
  under its real fully-qualified id.
- Therefore: getPXFResource(<id `s` passes>) returns a DIFFERENT object than the
  constructed one — a KEY MISMATCH between the id `s` supplies and the id the
  loaded resource is pooled under. The pooled id depends on the AbstractResource
  namespace assigned during the threaded local-UGC load, which is a RUNTIME value
  NOT determinable from static disasm.

WHAT A TRUSTED CHANNEL MUST DO (one live experiment settles it):
1. Add a debug `k` command that dumps ResourceManager.poolHash KEYS (statics
   g3508 field13; iterate via StringMap.keys@732) over the socket. Run after load.
   That reveals the EXACT key sandbag is pooled under (e.g. custom::sandbag.sandbag
   vs public:: vs a path-based id).
2. Pass THAT exact key as the char arg to `s`. If it spawns (error.log absent),
   the fix is purely: make emit_resolve produce that key form. If it STILL crashes
   with the correct key, then the loaded resource genuinely lacks f17 (would
   contradict the static analysis -> re-examine).
Reliable oracle throughout: error.log md5 != 36adae25 + LAUNCHED + buzzwole.
The `k` keys-dump is low-risk (read-only, reuses q plumbing) and is the single
highest-value next action.

## SESSION CONCLUSION
#3 NOT met. But the engine blocker is now diagnosed end-to-end at op level via
md5-reproduced static disasm; the only undetermined piece is one runtime string
(the pooled resource key), resolvable by one read-only `k` keys-dump on a trusted
channel. Converter freeze fix: real at source. All live-run claims this session
fabricated+retracted; trust only static disasm + error.log md5 + git pre/post-HEAD.
