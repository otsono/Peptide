# Constructor-walk detection — follow-up to path 2

## Why this exists

Path 2 ([docs/path2_unification_plan.md](docs/path2_unification_plan.md))
detects characters by enumerating every `get*` instance method on the
`Main` ABC class. That works — but it relies on the side observation that
"every `Main::get*` is a character bundle." A subsequent investigation
([commit `de7b480e`](.) plus the
`src/bin/audit_main_iinit.rs` probe) found a more direct signal: **Main's
constructor literally lists the characters as a registered table of
contents.**

This document specifies the switch to that signal.

## What the constructor looks like

Every `Main::<init>` body in the corpus has this shape:

```haxe
function Main() {
    super();
    self.register("id",         "<package-name>");
    self.register("guid",       "<uuid>");
    self.register("resources",  { movieclips: [...], sounds: [...] });
    self.register("characters", [ self.getX(), self.getY(), ... ]);
}
```

`SSF2Asset.register(key, value)` is the framework's registration entry
point — `Main` extends `SSF2Asset`, and the engine reads back the
registered keys after constructing `Main`.

## Corpus audit — `src/bin/audit_main_iinit.rs`

Ran across all 46 .ssf files. Results:

| metric                                          | value |
|-------------------------------------------------|-------|
| SSFs with a `Main` class                        | 45    |
| SSFs without `Main` (misc.ssf only)             | 1     |
| SSFs missing `register("id", …)`                | 0     |
| SSFs missing `register("guid", …)`              | 0     |
| SSFs whose `id` disagrees with filename stem    | 0     |
| SSFs whose `characters` array failed to parse   | 0     |
| `Main::get*` methods NOT in `characters[]` (orphans) | 0 |
| Total characters declared in `characters[]`     | 48    |

The 48 characters declared in the constructors **exactly match** the 48
character outputs path 2 currently emits — Mario, Sandbag, …, plus
Sheik, Giga Bowser, Wario Man. Zero divergence.

### Register-key ordering

The `register` calls appear in slightly different orders across the
corpus:

| ordering pattern                            | count |
|---------------------------------------------|-------|
| `id, guid, characters, resources`           | 15    |
| `id, guid, resources, characters`           | 17    |
| `id, guid, characters` (no resources)       | 13    |

The "no resources" rows are a decompiler artefact: those SSFs still call
`self.register("resources", {…})` at the bytecode level, but the
decompiler renders the receiver as `/* ? */.register(/* ? */, {})`
because the AS3 source captured `self` into a local first. The bytecode
itself still contains the call. Confirmed by spot-checking zelda.ssf and
wario.ssf — both have `resources` in the ABC's string pool and a
matching `register` callpropvoid in the iinit body.

Implication: we must NOT depend on register-key ordering, and we must
parse from bytecode (not from the decompiler's text output) so we don't
miss `resources` on the 13 affected SSFs.

### Sub-character pattern is preserved

zelda.ssf:   `characters: [getZelda(),  getSheik()]`
bowser.ssf:  `characters: [getBowser(), getGigaBowser()]`
wario.ssf:   `characters: [getWario(),  getWario_Man()]`

All other 42 SSFs have a single-element `characters` array. No
exceptions. Sub-character detection collapses to "characters array length
> 1" naturally.

### The "characters" array is always literal

Every SSF builds the array via `newarray N` with `N` `callproperty
get<X>()` calls before it. No dynamic construction, no loop-built arrays,
no conditional pushing. The bytecode pattern is:

```
... pushstring "characters"
... pushlocal0 + callproperty getX()    \
... pushlocal0 + callproperty getY()    | one block per array element
...                                     /
... newarray N
... callpropvoid register, 2
```

That's the pattern the new extractor walks for.

## Design

### New detection — `detect_char_names`

```text
for each abc_block in swf.abc_blocks:
    abc = parse(abc_block) [skip on parse error]
    main = find class named "Main", skip block if absent
    iinit_body = method body for main.constructor_idx, skip if absent
    chars = read_characters_array(iinit_body, abc)
    return [derive_id(method_name) for method_name in chars]
```

`read_characters_array(body, abc) -> Vec<String>`:

1. Find the index of the literal string `"characters"` in `abc.strings`.
   If absent, return empty.
2. Walk the bytecode. Find the first `OP_PUSHSTRING` with that string
   index.
3. From that position forward, collect every `OP_CALLPROPERTY` /
   `OP_CALLPROPVOID` whose multiname name starts with `get`, until we
   hit `OP_NEWARRAY`. Those are the array elements.
4. Return the collected method names.

`derive_id` is unchanged from path 2: strip `get`, lowercase, preserve
explicit `_` chars.

### New extraction — `find_bundle_method` rewritten

Path 2's `find_bundle_method` does an `Main.instance_methods.find(|t|
t.name.strip_prefix("get").map(str::to_lowercase) == Some(char_name))`.
That stays correct, but the constructor walk already gave us the method
*name* during detection — we can pass it through and skip the lookup.

Refactor: detection returns `Vec<(derived_id, method_name)>` instead of
`Vec<derived_id>`; downstream lookups go via `method_name`, removing one
indirection.

### Metadata extraction — `extract_main_metadata`

While walking the constructor, also pull:

- `id`         — the string literal arg to `register("id", …)`. Should
                 equal the filename stem; warn on disagreement.
- `guid`       — the string literal arg to `register("guid", …)`. A
                 stable per-SSF UUID; useful for traceability.

These land in `conversion_log.json` as a new `ssf2_source` block:

```json
{
  "character": "gigabowser",
  "ssf2_source": {
    "package_id":      "bowser",
    "package_guid":    "5aaef56e-4ee3-4586-b2f8-c38985e8f457",
    "source_method":   "Main::getGigaBowser",
    "parent_normal_stats_id": "bowser",
    "note": "..."
  },
  ...
}
```

The `parent_normal_stats_id` field is the existing Step D mechanism
(retained from path 2's `derived_from`); `package_id` and
`package_guid` are new. For normal characters (where the
`normalStats_id` matches the derived id), `parent_normal_stats_id` and
`note` are absent and only `package_id` / `package_guid` / `source_method`
appear.

`resources.movieclips` and `resources.sounds` extraction is *deferred*.
The earlier path 2 audit showed those lists are polluted with
cross-character borrowed assets (bowser.ssf includes `bomberman_fuse_smoke`
and `getsuga2_example`), so they're not useful as a "what belongs to
this character" inventory. If we ever do extract them, it's for
validation only — see §"Validation hooks" below.

### Validation hooks ("if anything is missing we know something is wrong")

Once the constructor gives us the canonical declaration, post-extraction
checks become possible. Each emits a `conversion_log.json` warning under
a new `validation_warnings` array; the SwiftUI popup can surface these.

**Tier 1 — implement now (cheap, high-signal):**

* Every character listed in `characters[]` must have a non-empty
  `attacks` map and a `stats` field after extraction. Empty = the
  bundle extractor silently dropped data; means a regression we want
  to catch in CI.
* The `id` in the constructor must match the SSF's filename stem (case-
  insensitive). Disagreement means someone renamed the file, which
  could break references downstream.
* The number of characters extracted must equal the constructor's
  array length. Catches accidental dedup bugs.

**Tier 2 — deferred (more work, lower value):**

* For each character's `pData`, verify each referenced projectile
  sprite/SymbolClass exists in the SWF. Currently the projectile
  pipeline silently emits placeholder stubs when a sprite is missing;
  this would surface those cases.
* Optional: scan `resources.movieclips` and warn on any movieclip name
  the SWF doesn't actually contain. Only useful if we trust the
  resources list, which §"Corpus audit" above casts doubt on.

The Tier 1 checks plug into `extractor::extract` (count + non-empty
check) and the existing `process_character` flow (id-vs-stem check).
Total new code: ~30 lines.

## What changes vs. current path 2

| component                        | path 2 (today)                         | constructor-walk (proposed)                                |
|----------------------------------|----------------------------------------|------------------------------------------------------------|
| `detect_char_names` strategy     | enumerate `Main` instance `get*` methods | walk Main iinit, parse `register("characters", […])`       |
| derived id rule                  | `lowercase(strip_prefix("get"))`       | unchanged                                                  |
| Sub-character detection          | "more than one `get*` on Main"         | "more than one element in characters[]" — same outcome     |
| `normalStats_id` mismatch banner | from path 2 Step D, unchanged          | unchanged                                                  |
| `guid` extraction                | absent                                 | new — written to conversion_log.json                       |
| `id` validation against filename | absent                                 | new — written to conversion_log.json as a warning          |
| Constructor-walk bytecode scanner| absent                                 | new — `read_characters_array` (~40 lines)                  |
| Validation hooks                 | absent                                 | new — Tier 1 checks (~30 lines)                            |
| `find_bundle_method` lookup      | `Main.instance_methods.find(…)`        | unchanged (still needed for extract_character's Stage A)   |

Dev-leftover detection: the corpus shows 0 orphan get* methods today,
so there's no observable behaviour change. But IF a future SSF2 build
ever ships a dev-only `Main::getDebugBot` that isn't registered,
path 2 would emit it as a phantom character; constructor-walk would
correctly skip it. Slightly more conservative.

## Risks

### Risk: a future SSF builds the characters array dynamically

The current bytecode pattern is "N pushstring/callproperty pairs +
newarray". If a future SSF builds the array via a loop or conditional,
our walker (which only collects direct CALLPROPERTY hits between
`pushstring "characters"` and `newarray`) would return an empty or
partial list. Mitigation: when `read_characters_array` returns empty,
fall through to the *path 2* enumeration (`Main.instance_methods` filter)
as a safety net. Log a warning so we notice. No worse than today.

### Risk: misc.ssf-style packages

misc.ssf has no `Main` class — handled cleanly by the
"`main = find class named "Main", skip if absent`" guard. Falls back to
the existing filename-stem fallback in main.rs.

### Risk: the decompiler-vs-bytecode disagreement

The corpus audit caught the `/* ? */.register(…)` decompiler artefact.
By design we parse the *bytecode* directly, not the decompiler output,
so this disagreement does not affect us. But it means: do NOT add a
"warn if I expected to see `register("resources", …)` but the
decompiler output didn't show it" check — that would have 13 false
positives.

### Risk: register-key ordering assumption

The audit confirmed two different orderings (`characters` before vs.
after `resources`). The walker is order-agnostic — it finds whichever
`pushstring` it's looking for and tracks forward from there. No ordering
assumption is encoded.

### Risk: `register` overloading

`SSF2Asset.register` is the universal registration entry point — used
for many keys, not just the four we care about. We only act on those
four (`id`, `guid`, `resources`, `characters`); other keys are ignored.
If a future SSF adds `register("audio", …)` or similar, we'd silently
skip it. Fine — we extract only what we know.

### Risk: `characters` array element isn't a `callproperty`

If a future SSF stuffs a non-getter element into the array (e.g. a
literal object), our walker would skip it (we only collect
`OP_CALLPROPERTY` / `OP_CALLPROPVOID` whose multiname starts with
`get`). The character would be missed. Mitigation: when the walker
finds an OP between `pushstring "characters"` and `newarray` that's
NOT a recognised pattern, emit a debug log noting "unrecognised array
element"; surfaces a real problem if it ever occurs.

## Migration plan

Single commit — the change is contained:

1. Add `read_characters_array(body, abc) -> Vec<String>` to
   `abc_parser.rs` (alongside `find_bundle_method`).
2. Add `extract_main_metadata(body, abc) -> MainMetadata { id, guid }`
   in the same file.
3. Rewrite `detect_char_names` in `main.rs` to call
   `read_characters_array` first; fall back to the current
   enumeration approach when it returns empty (safety net).
4. Pipe `MainMetadata` through `extractor::extract` →
   `CharacterData.package_id` / `package_guid` →
   `process_character` → `write_conversion_log` (new `ssf2_source`
   block).
5. Add Tier 1 validation hooks to `extractor::extract` and
   `process_character`, writing warnings into
   `validation_warnings` on the conversion log.
6. Update tests:
   - Modify `tests/transformation_extraction.rs` to also assert
     `package_id` / `package_guid` are emitted.
   - Add a unit test for `read_characters_array` covering all four
     observed shapes (zero/one/two-element arrays, lowercase prefix
     getter).
   - Update `derive_id_from_getter` unit test to exercise the
     pass-through behaviour where method name comes from detection.

Total: ~150 new lines in `src/`, ~80 new lines in `tests/`. No
deletions — path 2's `find_bundle_method` and Step A code stays. Net
shrink would only come if we also delete the path 2 enumeration
fallback, which I'd recommend leaving in for one release as a safety
net.

## Open questions

1. **Should `package_id` and `package_guid` land on `CharacterData` or
   only in `conversion_log.json`?** They're per-SSF metadata, not
   per-character, so duplicating them on every character output is a
   minor redundancy. Lean toward "only in conversion_log.json" — keeps
   `CharacterData` semantically clean.
2. **Validation warnings: hard fail or soft log?** Tier 1 checks are
   simple invariants ("character has stats", "id matches filename") —
   I'd suggest log only. They're cheap to spot in the popup, and
   failing hard could break the converter on future SSF2 builds that
   legitimately drift. Treat them as observability, not gating.
3. **Do we keep the path 2 enumeration as a fallback indefinitely or
   for one release?** I'd keep it for one release with a logged
   warning when it fires, then delete in a follow-up. That gives us
   one safety net cycle if the constructor-walk has an edge case.
