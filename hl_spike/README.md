# Haxe → .hl spike (feasibility result)

Goal: establish the greenlit "write Peptide feature logic in Haxe, compile to
`.hl`, load into the running engine" path, so complex features (hitbox readback,
`verify`, in-engine measurement) live in readable Haxe instead of hand/Rust-emitted
HashLink opcodes.

## What this spike established

- **haxe is installed** (`brew install haxe` → 4.3.7) and its HashLink target works:
  `haxe -hl spike.hl -main Spike` compiles `Spike.hx` → a valid `.hl` bytecode file.
- **hlbc reads the output**: `peptide spike.hl /dev/null inspect` →
  `version=4 functions=322 strings=342 types=394 globals=79`.

## The blocker (why this is a project, not a tonight task)

HashLink bytecode is **monolithic** — there is no runtime "load another `.hl`
module" facility, so "load into the engine" means **merging** the compiled `.hl`
into the engine's bytecode at patch time. The spike pins the cost: even a TRIVIAL
Haxe program pulls in the HL runtime/std lib → **322 functions / 394 types / 342
strings**. Merging that into the engine's ~26 000 functions / ~41 000 strings
requires a full **cross-module linker**:
  1. append the module's functions/types/strings/globals/ints/floats to the engine
     tables;
  2. rewrite EVERY index operand in every appended opcode (findex, type, string,
     global, int) to its new merged index;
  3. dedup shared runtime types/functions (String, Array, …) that already exist in
     the engine, or accept duplicates (semantics TBD);
  4. wire the injected dispatch to call the appended entry point.

`hlbc` gives read/write table access, so it's mechanically possible, but (2)+(3)
across two large modules is the real work — a dedicated linker, not a quick spike.

## Decision

Deferred. The **static parity harness** (`DUMP_PARITY` + `tools/parity_check.py`)
already met the parity goal (45/45 hitbox-stat parity) WITHOUT `.hl`, and
Rust-generated `Asm` blocks cover the field-read/method-call readbacks
(`physics`/`anim`). The `.hl` linker is the right next investment for in-engine
*behavioral* measurement (dummy hit → damage/knockback), which is the only
dimension `Asm` can't easily reach — build it as a focused linker project.
