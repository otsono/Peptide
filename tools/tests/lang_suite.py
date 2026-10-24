#!/usr/bin/env python3
"""lang_suite.py -- run the compiler's language suite in Fraymakers' own interpreter.

Each case is a small function whose AS3 result is known (see crates/ssf2-converter/src/compile/
suite.rs), compiled the way content is. The suite and the runtime's language layer
(runtime/core/as3.hx) are added to a converted fighter's script for one boot, the engine runs
every case, and the fighter's script is put back and packed again afterwards.

usage: tools/tests/lang_suite.py [fighter]      (default sandbag)
"""
import os, subprocess, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import verify as V


def main():
    cid = sys.argv[1] if len(sys.argv) > 1 else "sandbag"
    project = os.path.join(V.ROOT, "characters", cid, f"{cid}.fraytools")
    scripts = os.path.join(V.ROOT, "characters", cid, "library", "scripts")
    script = next((os.path.join(d, "Script.hx") for d, _, fs in os.walk(scripts)
                   if "Script.hx" in fs and "Projectile" not in d), None)
    if script is None:
        print(f"no converted {cid} to run the suite in"); return 2
    suite = subprocess.run([V.BIN, "ssf2", "compile", "--lang-suite"], capture_output=True, text=True, check=True).stdout
    as3 = open(os.path.join(V.ROOT, "crates", "ssf2-converter", "runtime", "core", "as3.hx")).read()
    runner = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "lang_suite_runner.hx")).read()
    original = open(script).read()
    try:
        with open(script, "w") as f:
            f.write(original + "\n" + as3 + "\n" + suite + "\n" + runner)
        subprocess.run([V.BIN, "pack", "--project", project], check=True, capture_output=True)
        f = V.Engine("lang-suite")
        f.boot(["--char", f"{cid},{cid}", "--stage", V.CHAR_STAGE_FM])
        f.tick(60)
        m = f.tell('e@p0 "S:" + __langSuite()')
        got = f.reply(m, "E:S:", 30)
        f.close(final=True)
    finally:
        with open(script, "w") as out:
            out.write(original)
        subprocess.run([V.BIN, "pack", "--project", project], capture_output=True)
    print(got if got is not None else f"no answer: {getattr(f, 'last_error', '')}")
    return 0 if got and got.startswith("all ") else 1


if __name__ == "__main__":
    sys.exit(main())
