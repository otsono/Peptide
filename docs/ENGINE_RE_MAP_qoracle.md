
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
