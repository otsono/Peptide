#!/usr/bin/env python3
"""Reconstruct a PRE-loop-fix ("buggy") sandbag.fra from the fixed one, for the
in-engine freeze A/B. Reverses tools/patch_fra_loops.py: removes the appended
`i = i + 1;` from removeAllEffects's while-loop (recreating the non-terminating
loop that freezes the engine) and fixes the 3-byte big-endian JSON-length header.

Usage: python3 tools/make_buggy_fra.py <fixed.fra> <out_buggy.fra>
Verifies the result re-parses and that removeAllEffects no longer increments.
This is a TEST artifact only — never install it as the shipped content.
"""
import json, sys

NEW = b"removeChild(effects.get()[i]);\\n\\t\\t\\t}\\n\\t\\t}\\n\\t\\ti = i + 1;\\n\\t}"
OLD = b"removeChild(effects.get()[i]);\\n\\t\\t\\t}\\n\\t\\t}\\n\\t}"


def main(src, dst):
    data = open(src, "rb").read()
    if data[4:5] != b"{":
        sys.exit("unexpected header")
    jlen = int.from_bytes(data[1:4], "big")
    n = data.count(NEW)
    if n != 1:
        sys.exit(f"expected exactly 1 fixed loop, found {n} (already buggy?)")
    buggy = data.replace(NEW, OLD)
    delta = len(buggy) - len(data)  # -18
    buggy = bytearray(buggy)
    buggy[1:4] = (jlen + delta).to_bytes(3, "big")
    # verify
    j2 = int.from_bytes(bytes(buggy[1:4]), "big")
    o2 = json.loads(bytes(buggy[4:4 + j2]).decode())
    bad = False
    for s in o2["scripts"]:
        v = s.get("value", "")
        if "removeAllEffects" in v:
            i = v.find("function removeAllEffects")
            body = v[i:v.find("\n}", i)]
            if "i = i + 1" in body:
                bad = True
    if bad:
        sys.exit("verification failed: increment still present")
    open(dst, "wb").write(bytes(buggy))
    print(f"OK wrote {dst} bytes={len(buggy)} delta={delta} (removeAllEffects now non-terminating)")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: make_buggy_fra.py <fixed.fra> <out_buggy.fra>")
    main(sys.argv[1], sys.argv[2])
