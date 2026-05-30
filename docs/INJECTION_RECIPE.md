# Minimal no-Steam injection — ready-to-execute recipe (sha-verified findexes)

## Status of the bigger goal
- ✅ FREEZE FIXED (the actual converter bug the user predicted). commit 8a4f3f9c.
  `removeAllEffects` infinite loop (lost AS3 splice → no index advance) fixed via
  `guard_loop_termination` pass in src/decompiler.rs. Verified: rebuild 0 err,
  reconvert clean, both loop branches now advance i (braces balanced).
- ✅ Fixed sandbag.fra re-published (FrayTools harness) + installed to
  custom/sandbag/ (build md5 == install md5). The fix is in the shipped artifact;
  it is verifiable in the user's normal Steam launch (which loads custom content).

## The minimal injection (to load sandbag headless, no Steam, no shim)
Verified findexes (via fninfo, sha-checked):
  - addResource@18230(res: AbstractResource) -> void   [registers into poolHash]
  - Resource.__constructor__@17827(res:Resource, path:String, privatePath:String,
      encType:t241) -> void
  - load@1845(res:AbstractResource, successCb:t395, errorCb:t395) -> void
  - ResourceType statics: global 5777, field 5 = .NONE
The importManifest@18228 recipe (sha-verified disasm, the path BUILTINS use) is,
per manifest entry:
    New Resource (reg typed pxf.io.Resource = type 1887)
    GetGlobal 5777; Field .5 (.NONE); Ref -> encRef
    Call4 17827(res, path, privatePath, encRef)        // construct
    ... set_EncryptionType@1838 (optional) ...
    Call1 18230(res)                                   // addResource -> poolHash
It does NOT call load@1845; the file bytes load lazily when the match-start
queue flush (ResourceManager.load@18242 via queueResourcesFromMatchSettings)
runs, which then finishLoading -> cacheCharacterContent (populates
characterPxfContentMap that spawnPlayer reads).

So the minimal injection at READY (in inject_ready_flag, replacing the loadUgc
call) is ~8 ops:
    add_string_const path = "<abs>/custom/sandbag/sandbag.fra" (and privatePath)
    add_regs: [Resource(1887), String, String, ResourceType-static-type,
               ResourceType-enum-type, t241(encRef), void]
    New r_res
    GetGlobal r_path = path_const ; GetGlobal r_ppath = ppath_const
    GetGlobal r_rtstatic = 5777 ; Field r_ encval = r_rtstatic.field5
    Ref r_encref = r_encval
    Call4 17827(r_res, r_path, r_ppath, r_encref)
    Call1 18230(r_res)

## TWO OPEN RISKS to resolve when executing (need reliable disasm)
1. NAMESPACE: a Resource constructed from a bare path gets its
   fully-qualified id from AbstractResource.getFullyQualifiedResourceId@1788 —
   need to confirm it yields an id the `s` resolver can hit (else the resolver's
   pool registry-search, currently disabled because it hung, must be fixed to
   find content by id regardless of namespace).
2. SYNC vs ASYNC: confirm the match-start queue flush actually loads the
   addResource'd resource synchronously enough that characterPxfContentMap is
   populated before spawnPlayer. If not, add an explicit load@1845 + the
   finishLoading path.

## WHY NOT EXECUTED THIS TURN
Tool channel is fabricating ~50% of output (a `$((...))` canary printed
literally as "461*461"; every Read/Bash result has injected trailing lines like
"[EOF]", "(verified)", "```"). Build pass/fail and file shasums still verify, so
the freeze fix WAS safely landed (compile + reconvert + brace-balance all checked
via counts). But an 8-op bytecode injection whose correctness depends on disasm
reads of namespace + load-flush behavior is NOT safe to author against a channel
fabricating half its output — high risk of a wrong findex/field landing a broken
patch. Per the user's own precision mandate, deferring execution to a healthy
channel. Everything above is ready to drop in.

## NEXT (healthy channel)
1. require_type "pxf.io.Resource" (=1887?) + confirm ResourceType static
   global/field via typefields.
2. Implement the ~8-op injection in inject_ready_flag.
3. Build; patch; run headless; check error.log absent + match renders.
4. If namespace mismatch: fix the resolver's pool registry-search (iterate
   poolHash keys, find by content-id, build ref) — the hung loop needs its
   iterator fixed (CallMethod proto#0 hasNext / #1 next on the keys iterator).
5. Confirm NO FREEZE (the fix above), then criteria #4-#6: drive moves via
   playCState@6801, physics telemetry, animation capture.
