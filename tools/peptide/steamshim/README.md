# Steam interpose shim (headless UGC load, no Steam client)

Fraymakers' `./hl` reports `[API loaded no]` when launched directly (our patched
bytecode path), so the Steam UGC pipeline never loads custom/ + workshop content
→ converted characters can't spawn. This shim forces the Steam "is running / init
OK / don't restart" answers positive so the real UGC pipeline runs.

## Build (x86_64 — the engine is x86_64, runs under Rosetta on Apple Silicon)
    clang -arch x86_64 -dynamiclib -o steamshim.dylib steamshim.c \
      -L"<Fraymakers dir>" -lsteam_api

## Use — IMPORTANT: hl has hardened runtime (flags=0x10000), which makes macOS
STRIP DYLD_INSERT_LIBRARIES. So you must run an ad-hoc re-signed copy of hl with
the runtime flag cleared:
    cp hl hl_shim
    codesign --remove-signature hl_shim
    codesign -f -s - hl_shim          # ad-hoc, no --options runtime
    DYLD_INSERT_LIBRARIES=steamshim.dylib DYLD_LIBRARY_PATH=. ./hl_shim _conn.dat

Never modifies the user's installed hl in place — operates on hl_shim copy,
removed after the run (same pattern as _conn.dat / steam_appid.txt).
