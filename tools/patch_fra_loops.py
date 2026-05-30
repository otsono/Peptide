#!/usr/bin/env python3
"""Deterministic .fra loop-termination patcher (no FrayTools needed).

A Fraymakers .fra is: 1 header byte + 3-byte big-endian top-level-JSON length +
the JSON + a trailing binary blob. The JSON's scripts[].value fields hold the
Haxe SOURCE as plain text (Fraymakers runs it via hscript at load time). Blob
offsets inside the JSON are blob-relative, so resizing the JSON is safe as long
as the blob bytes stay byte-identical and contiguous after it.

This patches the known non-terminating SSF2 array-walk loop in `removeAllEffects`
(the converter freeze: the loop's else-branch never advanced `i`, so it spun
forever every frame via its LINK_FRAMES listener). It inserts one `i = i + 1;`
at the end of the while body via a byte-level replace, then fixes the 3-byte
length header by the exact delta — preserving all original formatting.

This is a stopgap for when FrayTools republish can't be used; the proper fix is
in the converter (src/decompiler.rs counter_of_cond, which now matches Local(n)
counters), so freshly published .fra files don't need it.

Usage: python3 tools/patch_fra_loops.py <path/to/character.fra>
Verifies the result end-to-end and prints a report; exits non-zero on failure.
"""
import json, sys

OLD = b"removeChild(effects.get()[i]);\\n\\t\\t\\t}\\n\\t\\t}\\n\\t}"
NEW = b"removeChild(effects.get()[i]);\\n\\t\\t\\t}\\n\\t\\t}\\n\\t\\ti = i + 1;\\n\\t}"


def patch(path):
    data = open(path, "rb").read()
    if data[4:5] != b"{":
        sys.exit("ERROR: unexpected header (JSON does not start at byte 4)")
    jlen = int.from_bytes(data[1:4], "big")
    blob = data[4 + jlen:]
    n = data.count(OLD)
    if n == 0:
        # Either already patched or this .fra has no such loop.
        if NEW in data:
            print("ALREADY_PATCHED")
            return
        sys.exit("ERROR: target loop pattern not found")
    new = data.replace(OLD, NEW)
    delta = len(new) - len(data)
    new = bytearray(new)
    new[1:4] = (jlen + delta).to_bytes(3, "big")
    # Verify before writing.
    j2 = int.from_bytes(bytes(new[1:4]), "big")
    o2 = json.loads(bytes(new[4:4 + j2]).decode("utf-8"))
    fix_ok = any("removeAllEffects" in s.get("value", "") and "i = i + 1;\n\t}" in s["value"]
                 for s in o2["scripts"])
    blob_ok = bytes(new[4 + j2:]) == blob
    if not (fix_ok and blob_ok):
        sys.exit(f"ERROR: verification failed (fix={fix_ok} blob={blob_ok})")
    open(path, "wb").write(bytes(new))
    print(f"PATCHED occurrences={n} delta={delta} new_bytes={len(new)} fix_ok={fix_ok} blob_intact={blob_ok}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit("usage: patch_fra_loops.py <character.fra>")
    patch(sys.argv[1])
