
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
