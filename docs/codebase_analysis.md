# SSF2 → Fraymakers converter — codebase analysis

> **Snapshot date / status banner — written pre-Phase-2 / pre-path-2,
> updated against HEAD `d8e328af`:**
>
> Several large items in this audit have since been implemented or
> deleted. Quick status by section so a reader doesn't act on stale
> recommendations:
>
> - §1.1 / §1.2 *Parse the SWF once*: **done** (commit `5f34c666`).
> - §1.3 *Unify the AVM2 stack simulators*: **done** (commit `21562ab6`).
> - §1.5 / §1.7 *Per-multiname clones / `wrap_persistent_state` regex
>   caching*: §1.7 **done** (commit `e7e62111`); §1.5 still open.
> - §2.1 *Delete dead `build_*_map` block*: **DEFERRED** — the functions
>   are marked TODO and survive until JSONC parity is confirmed
>   (commit `43a13638`).
> - §2.2 *Delete dead costume / stat extractors*: **done** (commit
>   `7671defe`).
> - §2.3 *Hand-rolled SWF tag walker in sound_extractor*: still open.
> - §3.5 *`getproperty` mishandling*: **done** as part of the §1.3 visitor
>   unification.
> - §3.7 *`infer_ext_var_types` floats-as-Int*: **done** (Phase 2 bug fixes).
> - §3.30 *entity_gen UUID collision*: still open; defensive comment
>   added.
> - The §0 orientation paragraph below still describes the *pre-path-2*
>   detection ("finds the per-character `XxxExt` AS3 class"). Current
>   detection walks `Main`'s constructor — see
>   [`DEVELOPMENT.md`](../DEVELOPMENT.md) §4 and §5.1, and
>   [`constructor_walk_detection.md`](constructor_walk_detection.md).
>
> Treat this document as a historical reference that's still useful for
> the **shape** of the codebase, but verify any specific
> recommendation against `git log` before acting on it.

## Orientation

The converter is a single-binary Rust CLI (`ssf2_converter`) plus a SwiftUI wrapper. It opens an SSF2 `.ssf` (an SSF-wrapped, zlib-compressed SWF) and emits a Fraymakers character package (`.fraytools` project file, library tree of `.entity`, `.hx`, `.palettes`, sprites, sounds). The flow per character in [main.rs](src/main.rs):

1. **`ssf::decompress` + `swf_parser::parse`** — produce a `SwfFile { version, frame_count, frame_rate, symbols, abc_blocks }`.
2. **`extractor::extract` → `abc_parser`** — parses each `DoAbc` block, finds the per-character `XxxExt` AS3 class, walks its methods, extracts `getOwnStats`/`getAttackStats`/`getProjectileStats` via a hand-rolled AVM2 stack simulator, and decompiles every other Ext-class method via `decompiler.rs` (a CFG-reconstructing AVM2 → Haxe decompiler).
3. **`sprite_parser`** — walks SWF DefineSprite tags to extract per-animation, per-frame collision-box geometry, plus root-MovieClip `stance` transforms (`extract_xframe_transforms` + `extract_xframe_scale`).
4. **`image_extractor`** — extracts every DefineBitsLossless/JPEG3 bitmap to PNG, builds shape→bitmap maps and animation-to-frame-image tables, pre-renders any sheared placements as baked PNGs, then discovers projectile, effect, and head sprites.
5. **`sound_extractor`** — parses DefineSound tags (Nellymoser / MP3 / ADPCM), wraps Nellymoser data in synthetic FLV containers, and shells out to `ffmpeg` for WAV conversion.
6. **`palette_gen`** — either uses real SSF2 costume data from misc.ssf (extracted via `abc_parser::scan_all_costume_methods` in main.rs `extract_costumes_to_temp`) or falls back to k-means on idle sprites.
7. **`entity_gen` + `haxe_gen`** — turn everything above into the dozens of JSON `.entity` files, Haxe stat / script files, manifest, palette files, projectile/effect/menu sub-entities, and `.meta` sidecars.

Data-driven config lives in [mappings/commands.jsonc](mappings/commands.jsonc) (universal SSF2→FM API translations: literal replacements, regex replacements, `call_splits`, `frame_params`/`isframe`, `passthrough_fm_apis`, `ssf2_only`, `attach_effect_props`, `global_vfx_map`) and [mappings/character/](mappings/character/) (`stats.jsonc`, `hitbox_stats.jsonc`, `animations.jsonc`). The loader in [mappings.rs](src/mappings.rs) strips JSONC, picks the on-disk override or the `include_str!`'d default, and caches via `OnceLock`. `api_mappings::translate_ssf2_to_fm` is the post-decompile text pipeline that runs these tables against decompiled Haxe.

The pipeline shape is "parse SWF many times, scan all bytecode many times, then emit a lot of JSON" — there's substantial duplicated parsing work and several large stack-simulator copies that could be unified.

---

## 1. Optimization opportunities

Rough rubric: **impact** = how much wall-time / output bytes the change saves; **effort** = small/medium/large.

### 1.1 [HIGH impact / small effort] Parse the SWF once, not 6–13 times per character
[main.rs:78](src/main.rs#L78), [main.rs:254](src/main.rs#L254), [main.rs:264](src/main.rs#L264), [main.rs:273](src/main.rs#L273), [main.rs:294](src/main.rs#L294), [sprite_parser.rs:194-195](src/sprite_parser.rs#L194), [sprite_parser.rs:291-292](src/sprite_parser.rs#L291), [sprite_parser.rs:315](src/sprite_parser.rs#L315), [sprite_parser.rs:778-779](src/sprite_parser.rs#L778), [sprite_parser.rs:1039-1040](src/sprite_parser.rs#L1039), [image_extractor.rs:181-183](src/image_extractor.rs#L181), [image_extractor.rs:308](src/image_extractor.rs#L308), [image_extractor.rs:1259-1260](src/image_extractor.rs#L1259), [image_extractor.rs:1510-1511](src/image_extractor.rs#L1510).

Every entry point re-runs `swf::decompress_swf` + `swf::parse_swf` on the same `swf_data` buffer. Per character, today's call graph is roughly:

- `swf_parser::parse` (1×, in main)
- `sprite_parser::extract_xframe_scale` (1×)
- `sprite_parser::parse_sprite_boxes` (1×) — which **also** internally calls `extract_xframe_transforms` (another parse)
- `image_extractor::extract_images` (1×) — which also calls `extract_xframe_transforms` (yet another parse)
- `image_extractor::discover_projectiles_and_head` (1×)
- `image_extractor::extract_projectile_frame_images` (1× per projectile)
- `sprite_parser::extract_boxes_for_sprite_id` (1× per projectile)
- `image_extractor::extract_projectile_frame_images` again (1× per effect, via `entity_gen::generate_effect_entity`)

For a character with 5 projectiles and 8 effects (Mario-ish), that's ≈ 7 + 5 + 5 + 8 = **25 full SWF decompress+parse passes** on the same bytes. `swf::parse_swf` allocates the entire tag tree each time and isn't cheap.

Fix: parse once in main, thread the parsed `swf::Swf` (or our `SwfFile`) through every callee instead of `swf_data: &[u8]`. The shape of the existing `SwfFile { abc_blocks, symbols, … }` already proves the discipline is feasible; extend it to carry the parsed tags (or pass the raw `swf::Swf` alongside).

Even a partial change — pass `&swf::Swf` to `image_extractor::extract_projectile_frame_images` and `sprite_parser::extract_boxes_for_sprite_id` so they don't re-parse per projectile/effect — would knock per-character runtime down materially for projectile-heavy chars.

### 1.2 [HIGH impact / small effort] Don't extract `xframe_transforms` twice per character
[sprite_parser.rs:315](src/sprite_parser.rs#L315), [image_extractor.rs:308](src/image_extractor.rs#L308).

`extract_xframe_transforms` decompresses+parses the SWF and walks every DefineSprite tag. It's invoked from inside both `parse_sprite_boxes` (always) and `extract_images` (always). The two callers produce identical maps. Pull the call up to main, compute the map once, pass it in.

### 1.3 [HIGH impact / medium effort] Unify the four near-identical AVM2 stack simulators
[abc_parser.rs:1244 `extract_attack_objects`](src/abc_parser.rs#L1244), [abc_parser.rs:1435 `extract_projectile_objects`](src/abc_parser.rs#L1435), [abc_parser.rs:1650 `extract_stats_from_body`](src/abc_parser.rs#L1650), [abc_parser.rs:1741 `extract_ssf2_stats`](src/abc_parser.rs#L1741), [abc_parser.rs:1827 `extract_largest_numeric_object`](src/abc_parser.rs#L1827) (currently `#[allow(dead_code)]`), [abc_parser.rs:2099 `extract_costume_data`](src/abc_parser.rs#L2099), [abc_parser.rs:2330 `decode_costume_objects`](src/abc_parser.rs#L2330), [decompiler.rs:706 `BlockDecoder::decode_range`](src/decompiler.rs#L706).

Six (or seven, counting the dead one) full hand-rolled AVM2 mini-interpreters live in this codebase, all with the same `match op` shape. Each handles ~20 opcodes; each duplicates the constant-pool reads (`pushstring → strings.get(idx)`, `pushdouble → doubles.get(idx)`, …), `newobject` key-extraction, branch skipping, the `if stack.len() > 256 { stack.drain(0..128); }` safety hack, etc.

A single shared "scan ABC bytecode" iterator (`fn for_each_instruction(bc, abc, &mut FnMut(Op, &mut StackVal))`) plus per-call-site callbacks would let `extract_attack_objects` / `extract_projectile_objects` / `extract_ssf2_stats` / `decode_costume_objects` collapse to a fraction of their current size and ~halve the total source. The two costume-extracting functions in particular are almost copy-paste duplicates of each other and of `extract_attack_objects`.

This is also a meaningful binary-size win — each `match` arm gets monomorphized into a unique code path.

### 1.4 [MEDIUM impact / small effort] `scan_all_costume_methods` runs `decode_costume_objects` on every method body
[abc_parser.rs:2315](src/abc_parser.rs#L2315).

```rust
for body in &abc.method_bodies {
    let per_char = decode_costume_objects(&body.bytecode, abc);
    …
}
```

`misc.ssf` has thousands of method bodies. Pre-filter to bodies that contain *any* `pushstring "Default"` or `paletteSwap` reference (a simple byte-substring scan over the bytecode against the string indexes used by the `paletteSwap` / `colors` / `replacements` keys) before running the full stack sim. Typical speedup is the ratio of all-bodies to costume-bodies — likely ≥ 50× on misc.ssf.

### 1.5 [MEDIUM impact / small effort] Eliminate per-multiname / per-method `.clone()` allocations during ABC parse
[abc_parser.rs:462-475](src/abc_parser.rs#L462), [abc_parser.rs:559](src/abc_parser.rs#L559), [abc_parser.rs:684-691](src/abc_parser.rs#L684), [abc_parser.rs:953](src/abc_parser.rs#L953).

The ABC parser allocates a fresh `String` clone for every multiname's name (`strings.get(name_idx).cloned()`) — once during parse, then again into each `Class.name` / `Trait.name` / `Method.name`. For a real-world ABC that's tens of thousands of `String` clones. Three options, in order of disruption:

- Keep `Multiname.name` as `Arc<String>` or `Rc<String>` from a shared pool.
- Store only `name_idx: u32` and look up against `&abc.strings` on demand (this is what AVM2 itself does).
- Switch the whole abc-parser to borrowed `&'a str` against the input buffer (the swf crate gives this already; only the SSF2 path uses owned strings).

Option 2 is the smallest change: `Multiname { name_idx }`, plus a helper `abc.multiname_name(idx) -> &str`. Then `Class { name: u32 (multiname_idx) }`. The downside is touching every read site, but they're mechanical replacements.

### 1.6 [MEDIUM impact / small effort] `apply_one_call_split` rescans the whole file once per call-split entry
[api_mappings.rs:1036-1043](src/api_mappings.rs#L1036).

```rust
let mut current = code.to_string();
for (source_method, split) in &cfg.call_splits {
    current = apply_one_call_split(&current, source_method, split);
}
```

Each iteration does a full O(n) walk of the (potentially large) script, building a new `String`. With N call_splits this is O(N · |code|). For one character that's not catastrophic but it's all unnecessary copies. Build all needles up-front, do a single forward pass, dispatching on whichever needle hits at the current cursor. Same algorithmic complexity gain as combining `replace`-loops in `translate_ssf2_to_fm`.

While you're there: every `for r in &cfg.replacements { result = result.replace(&r.from, &r.to); }` in [api_mappings.rs:546-548](src/api_mappings.rs#L546) is O(N · |code|) the same way. A single Aho–Corasick scan (the `aho-corasick` crate is already a transitive dep via `regex`) would do this in one pass.

### 1.7 [MEDIUM impact / small effort] `wrap_persistent_state` recompiles two regexes per ext_var on every script
[api_mappings.rs:1473](src/api_mappings.rs#L1473), [api_mappings.rs:1482](src/api_mappings.rs#L1482), [api_mappings.rs:1492](src/api_mappings.rs#L1492).

```rust
for (name, kind) in var_types {
    if *kind == ExtVarType::Int {
        let re_inc = regex::Regex::new(&format!(r"\bself\.{}\+\+", regex::escape(name))).unwrap();
        …
        let re_dec = regex::Regex::new(…).unwrap();
    }
    let assign_re = regex::Regex::new(…).unwrap();
    let read_re   = regex::Regex::new(…).unwrap();
}
```

`Regex::new` is expensive. This function runs once on Script.hx and **once per frame-script body** in entity_gen (see [entity_gen.rs:326-329](src/entity_gen.rs#L326)). For Mario-sized characters that's hundreds of calls — each recompiling 2–4 regexes per ext_var.

Cheap fix: cache compiled regexes in a `OnceLock<Vec<(String, ExtVarType, Compiled)>>` keyed by the `var_types` map, or just lift the regex construction outside the inner loop and pass the compiled set down. Even simpler: do this transformation with a single byte-scan pass that doesn't need regexes at all (the pattern is `self.<known_name>` with simple suffix checks).

### 1.8 [MEDIUM impact / medium effort] `prerender_skewed_frames` re-opens the source PNG from disk for every cache miss
[image_extractor.rs:417](src/image_extractor.rs#L417).

```rust
let src = match image::open(char_output_dir.join(&src_img.png_path)) { … };
```

The PNG was *just written* a few hundred lines earlier (in `extract_images`); we have the in-memory RGBA buffer. Hold onto it and pass it in, or keep an `Option<RgbaImage>` cache on the `ExtractedImage` struct so subsequent skew-bakes for the same source bitmap don't re-PNG-decode it. PNG decode is the expensive step here; the bicubic loop is fast.

### 1.9 [MEDIUM impact / small effort] `find_collision_box_base_size` rebuilds its tally on every animation
Actually it doesn't — it's called once. Disregard. (Verified [sprite_parser.rs:307](src/sprite_parser.rs#L307).)

### 1.10 [LOW–MEDIUM impact / small effort] `extract_character` rebuilds `body_by_method` / `trait_name_for_method` / `method_names` once per ABC block
[abc_parser.rs:650-697](src/abc_parser.rs#L650).

OK because there's usually one ABC block per character SWF, but the `for (block_idx, abc_data) in swf.abc_blocks.iter()` in [extractor.rs:108](src/extractor.rs#L108) reparses *and rebuilds these indexes* per block. Caching them on the `AbcFile` would be neater and prevent later regressions.

### 1.11 [LOW impact / small effort] `prerender_skewed_frames` allocates two large `Vec<[f64; 4]>` for every baked frame
[image_extractor.rs:531-555](src/image_extractor.rs#L531).

The Gaussian's `tmp` and `blurred` are full-size float buffers — 32 bytes × dst_w × dst_h. For a 4096×4096 limit (which the code allows for) that's a 256 MB temporary on a single frame. Real bakes are tiny, so this is unlikely to bite in practice, but it'd be cheap to either drop the unsharp pass entirely (the user can sharpen later) or pre-allocate once per `prerender_skewed_frames` call.

### 1.12 [LOW impact / small effort] `entity_gen::generate_entity` collects `layer_kfs`/`kf_has_symbol` maps via two extra full passes over `keyframes`/`layers`
[entity_gen.rs:909-924](src/entity_gen.rs#L909).

These are built only to do the empty-animation drop. Once the entity is sized (thousands of keyframes per character), this allocates two BTreeMaps with thousands of entries. Threading the "has-image" bit through the per-animation loop directly avoids the maps entirely.

### 1.13 [LOW impact / small effort] `extract_xframe_name` reads u30 for `OP_PUSHBYTE`
[abc_parser.rs:623](src/abc_parser.rs#L623).

`OP_PUSHBYTE` has a literal one-byte operand, not a u30. The code is correct (`{ i += 1; }`), but the very next branch lumps PUSHSHORT/PUSHINT/PUSHUINT/PUSHDOUBLE — three of those (INT/UINT/DOUBLE) are constant-pool indexes (u30) and the fourth (SHORT) is a u30-encoded signed value. All four work as u30 reads, but a comment to that effect would help future readers; the visual lump suggests they're all the same kind of operand when they aren't.

---

## 2. Cleanup opportunities

### 2.1 Dead public surface in `api_mappings.rs`
[api_mappings.rs:40](src/api_mappings.rs#L40), [api_mappings.rs:280](src/api_mappings.rs#L280), [api_mappings.rs:298](src/api_mappings.rs#L298), [api_mappings.rs:369](src/api_mappings.rs#L369), [api_mappings.rs:388](src/api_mappings.rs#L388), [api_mappings.rs:1664](src/api_mappings.rs#L1664).

These six functions are `pub fn` but **never called from anywhere in the workspace**:
- `build_method_map` — 250+ lines of hardcoded SSF2→FM method mappings.
- `build_property_map` — small map of x/y/scaleX/scaleY/etc. property mappings.
- `build_state_map` — ~60 entries mapping IDLE/STAND/WALK/etc. to CState constants.
- `build_event_map` — small map of event names.
- `build_hitbox_prop_map` — small map of hitbox property names.
- `load_api_methods_json` — a hand-rolled JSON scanner reading `api_methods.json` (which doesn't exist anywhere in the repo).

The entire SSF2→FM API translation now flows through `mappings/commands.jsonc` + the small built-in `lookup_api` table in [decompiler.rs:20](src/decompiler.rs#L20). The legacy maps above are pure dead weight (~350 lines of source). Delete and remove the `MethodMapping` / `ArgTransform` types that go with them.

### 2.2 Dead helper code in `abc_parser.rs`
[abc_parser.rs:1827 `extract_largest_numeric_object`](src/abc_parser.rs#L1827) (already `#[allow(dead_code)]`), [abc_parser.rs:1884 `extract_frame_actions`](src/abc_parser.rs#L1884) (already `#[allow(dead_code)]`), [abc_parser.rs:2006 `is_attack_object`](src/abc_parser.rs#L2006), [abc_parser.rs:2015 `is_stats_object`](src/abc_parser.rs#L2015), [abc_parser.rs:2099 `extract_costume_data`](src/abc_parser.rs#L2099), [abc_parser.rs:2203 `extract_costume_data_from_apply_palette`](src/abc_parser.rs#L2203). Also [extractor.rs:355 `extract_animation_name`](src/extractor.rs#L355) (`#[allow(dead_code)]`).

`scan_all_costume_methods` is the live costume path (used from main); the two `extract_costume_data*` functions are obsolete competitors with no callers. ~250+ lines.

`extract_largest_numeric_object` is annotated dead. `extract_frame_actions` is annotated dead. The hand-rolled `extract_costume_data` function (around line 2028) is the unused early prototype — its diagnostic-only branch starting at line 2045 (`Decode the 13-byte wrapper…`) walks the bytecode just to log a few names; nothing else consumes it.

`ExtractedCharacter.costumes` (defined [abc_parser.rs:166](src/abc_parser.rs#L166)) is set to `vec![]` on line 1207 and never read by anyone outside this file — the actual costume extraction goes through `scan_all_costume_methods` directly in main.rs. Remove the field.

### 2.3 Hand-rolled SWF tag walker in `sound_extractor.rs` parallels the `swf` crate
[sound_extractor.rs:55](src/sound_extractor.rs#L55).

`parse_sounds` re-implements SWF tag iteration from raw bytes, presumably because at some point the `swf` crate didn't expose DefineSound. It does today. Replacing the hand-rolled parser would (a) delete ~80 lines, (b) eliminate a second separate parse of the same SWF buffer, and (c) remove an `swf[pos+i]` slice-index-vulnerable section.

### 2.4 Stub/empty fields and unused locals
- [extractor.rs:240](src/extractor.rs#L240): `let _ssf2_names: ...` — computed and immediately discarded.
- [decompiler.rs:1014](src/decompiler.rs#L1014): `OP_JUMP => { if pos + 3 <= self.bc.len() { let _ = pos + 3; } return None; }` — `pos + 3` is computed and dropped. Also at line 998: `let _ = pos;`. These are dead expressions disguised as comments. Either drop them or replace with a real `pos += 3`.
- [extractor.rs:355 `extract_animation_name`](src/extractor.rs#L355): `#[allow(dead_code)]`, never used.
- `BlockDecoder::clear_dup_residue` ([decompiler.rs:674](src/decompiler.rs#L674)) — empty function body, only the docstring remains. Delete.
- `StackVal::Bool` carries `()` ([abc_parser.rs:1235](src/abc_parser.rs#L1235)) with a code-comment "unused field — kept for stack value compatibility". The `()` is just visual noise; change to a unit variant.

### 2.5 Duplicate fallback tables
[sprite_parser.rs:549-580 `apply_fallbacks`](src/sprite_parser.rs#L549) and [image_extractor.rs:1075-1102 `apply_image_fallbacks`](src/image_extractor.rs#L1075) carry **the same fallback table** (stunned→hurt, fly→jump_aerial, swim→fall, ladder→idle, …), with only minor diffs. Lift the table to a shared module (or move to `mappings/`) and have both call it.

### 2.6 Inline test-only `FrameFnPattern` is theatre
[api_mappings.rs:874-893](src/api_mappings.rs#L874).

```rust
struct FrameFnPattern;
fn simple_frame_fn_re() -> FrameFnPattern { FrameFnPattern }
fn iter_frame_fns<'a>(code: &'a str, _pat: &FrameFnPattern) -> impl Iterator<…>
fn parse_frame_fn_header(trimmed: &str, _pat: &FrameFnPattern) -> Option<…>
```

The `_pat: &FrameFnPattern` argument is always ignored — the struct is a zero-sized stub left from when this was a real regex. Delete the wrapper type and the arg.

### 2.7 Redundant fallback parameter `_char_lower` in `extract_ssf2_anim_name`
[sprite_parser.rs:700-702](src/sprite_parser.rs#L700). Underscore-prefixed parameter, threaded through every caller, never used. Either use it for character-scoped matching or drop it.

### 2.8 Repeated symbol-table extraction
Almost every entry point starts the same way:
```rust
let mut symbols: BTreeMap<u16, String> = BTreeMap::new();
for tag in &swf.tags {
    if let swf::Tag::SymbolClass(links) = tag {
        for link in links {
            let name = link.class_name.to_str_lossy(…).to_string();
            symbols.insert(link.id, name);
        }
    }
}
```
See [image_extractor.rs:187-195](src/image_extractor.rs#L187), [image_extractor.rs:1262-1271](src/image_extractor.rs#L1262), [image_extractor.rs:1513-1524](src/image_extractor.rs#L1513), [sprite_parser.rs:197-205](src/sprite_parser.rs#L197), [sprite_parser.rs:294-303](src/sprite_parser.rs#L294), [sprite_parser.rs:781-789](src/sprite_parser.rs#L781), [sprite_parser.rs:1042-1050](src/sprite_parser.rs#L1042), and also `swf_parser::parse`. The existing `SwfFile.symbols` already has it. Plumb it through (or have the symbol map live alongside the parsed `swf::Swf` per §1.1).

### 2.9 Two near-identical "double for 30→60 fps" call sites
[entity_gen.rs:150-156 `double_keyframe_lengths`](src/entity_gen.rs#L150) doubles all keyframe lengths. `api_mappings::double_frame_counts` doubles specific call-arg / object-field literals. Both are data-driven from `mappings/commands.jsonc :: frame_params` / `hitbox_stats.jsonc :: isframe`. There's no fundamental redundancy but the comments at [entity_gen.rs:142-149](src/entity_gen.rs#L142) explaining "why both pipelines exist" don't make it obvious that one is text-based and the other is JSON-based; worth a brief block-comment unifying the story.

### 2.10 Two duplicate `extract_frame_labels` helpers
[sprite_parser.rs:420 `extract_frame_labels`](src/sprite_parser.rs#L420) and [image_extractor.rs:1058 `extract_frame_labels_from_sprite`](src/image_extractor.rs#L1058) are byte-identical. Have image_extractor call the sprite_parser one (or move to a shared location).

### 2.11 `decompile_method` / `decompile_closure` near-duplication
[decompiler.rs:1501 `decompile_closure`](src/decompiler.rs#L1501) and [decompiler.rs:1651 `decompile_method`](src/decompiler.rs#L1651) share the structure: build activation slots, set up `StructuredDecoder`, push param locals, decode, filter `Stmt::VarDecl(0, This)`, collapse duplicate ifs. They differ only in (a) the param-name source and (b) whether they call `rename_loop_counters`. Trivial extract-method opportunity.

### 2.12 Stale `// TODO` / preserved-original markers
- The lookup_api `note` field in [decompiler.rs:13-122](src/decompiler.rs#L13) is now mostly empty strings since the API translation moved to `commands.jsonc`. The `comment` is appended to the rendered call, which clutters output (`self.toState(…) // SSF2: …`). Either delete the field or pull these notes into the JSONC config too.
- `extract_costume_data` line 2042 has a `log::info!("getCostumeData wrapper: …")` and a "Scan ALL method bodies with many pushuint" block ([abc_parser.rs:2069-2086](src/abc_parser.rs#L2069)) that *just logs* what it would do, then discards `best`. This is debugging-leftover noise that runs on every misc.ssf load.
- [abc_parser.rs:2543 `// DEBUG REMOVED`](src/abc_parser.rs#L2543) trailing one-line comment-marker. Delete.
- `dump_*` binaries in [src/bin/](src/bin/) — there are 17 of them, each parsing the SWF independently. These look like one-time-debugging artifacts (`dump_aerial_down_frames`, `dump_trail_matrices`, etc.). If they're not still maintained, consider moving them under `examples/` or `experimental/` to keep them out of `cargo build`'s main path and make their experimental status clear.

### 2.13 Misspelled-but-load-bearing constant
[sprite_parser.rs:60-83 BoxType::from_instance_name](src/sprite_parser.rs#L60), [sprite_parser.rs:684](src/sprite_parser.rs#L684), [entity_gen.rs](src/entity_gen.rs) and others contain checks for both `collisonbox` and `collisionbox` because the SSF2 source files use the typo. This is correct — SSF2's actual symbol name is `CollisonBox` — but it'd be worth one central constant or helper (`is_collision_box_name`) shared across `sprite_parser`, `image_extractor`, and `entity_gen` so the misspelling lives in one place.

### 2.14 Animation expansion table is duplicated in code
[extractor.rs:248 `expand_split_anim`](src/extractor.rs#L248) hardcodes `jab → [jab1, jab2, jab3, jab4]` and `taunt → [taunt, taunt_up, taunt_down]`. The split logic itself lives in [anim_splitter.rs:486](src/anim_splitter.rs#L486) and [sprite_parser.rs:479 `sub_anim_splits`](src/sprite_parser.rs#L479). All three should be one table.

### 2.15 `costumes` field on `ExtractedCharacter` is always empty
[abc_parser.rs:166](src/abc_parser.rs#L166), [abc_parser.rs:1207](src/abc_parser.rs#L1207). See 2.2 — it's set to `vec![]` and never inspected. Drop the field and the `CostumeData` import in `extractor.rs`.

### 2.16 Hard-coded SSF2 attack name table next to `mappings/`
[abc_parser.rs:1950 `is_attack_name`](src/abc_parser.rs#L1950) and [abc_parser.rs:1966 `normalize_attack_name`](src/abc_parser.rs#L1966) are large hand-rolled tables that exactly duplicate parts of `mappings/character/animations.jsonc`'s `ssf2_to_fm`. Moving them to JSONC keeps every SSF2-name table in one place.

---

## 3. Potential bugs

For each, I give the file/line, the suspect code, and the smallest reproduction I can imagine. None are show-stoppers I directly confirmed by running the converter — they're things to verify.

### 3.1 `read_string` accepts non-UTF8 lossy without warning
[abc_parser.rs:289](src/abc_parser.rs#L289).

```rust
let s = String::from_utf8_lossy(&self.data[self.pos..self.pos + len]).to_string();
```

The ABC string pool is officially UTF-8, but SSF2 SWFs include some Windows-1252-ish strings (sound names, e.g. `naïve.mp3` if any non-ASCII slipped in). `String::from_utf8_lossy` silently inserts U+FFFD; later code uses `s.chars().all(|c| c.is_alphanumeric() || c == '_')` filters in [abc_parser.rs:617](src/abc_parser.rs#L617) and `looks_like_char_name` in [abc_parser.rs:2364](src/abc_parser.rs#L2364), which will treat the replacement character as not-alphanumeric and drop the whole string. That's safe but you'll get silent data loss for any non-UTF-8 trait name. Other ABC sites (`swf::SymbolClass`) use `to_str_lossy(WINDOWS_1252)` explicitly — abc_parser is inconsistent with the rest of the codebase.

**Repro hypothesis**: feed a character SWF whose `XxxExt` class has a trait named with a non-UTF8 byte (e.g. a `0xE9`/`é`). Today: the trait disappears from `ext_methods`. Either the ABC parser should match swf's convention (WINDOWS_1252), or warn when `from_utf8_lossy` actually replaced characters.

### 3.2 `BoxType::from_instance_name` operator-precedence bug for grabbox
[sprite_parser.rs:63-65](src/sprite_parser.rs#L63):

```rust
} else if lower.starts_with("grabbox") || lower.starts_with("grab") && lower.ends_with("box") {
    Some(BoxType::GrabBox)
```

`&&` binds tighter than `||`, so the expression is `starts_with("grabbox") || (starts_with("grab") && ends_with("box"))`. The author probably intended the parenthesised form already, but it's worth re-reading: the second branch also matches `grabholdbox`, `grabbedbox`, etc. and routes them to `GrabBox`. The next branch down explicitly maps `touchbox → GrabHoldBox` — but what about names like `grabHoldBox`? They start with `grab` and end with `box`, so they hit the GrabBox branch first instead of the GrabHoldBox check that comes later. SSF2 uses `touchBox` for grab-hold so this probably doesn't trigger today, but if SSF2 source ever has a `grabHoldBox` it'd misclassify.

### 3.3 `fm_box_index` strips leading-digit indices wrong
[entity_gen.rs:100-108](src/entity_gen.rs#L100):

```rust
fn fm_box_index(fm_name: &str) -> usize {
    fm_name.chars().rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars().rev()
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}
```

This works, but `"hitbox12"` → `"21"` reversed → `"12"` → 12, and `"hitbox"` → `""` → `parse().unwrap_or(0)` → 0. Fine. The function reverse-collects twice through `chars()`. Replace with `fm_name.trim_end_matches(|c: char| !c.is_ascii_digit()).chars().rev().take_while(|c| c.is_ascii_digit()).collect()` — or just `fm_name.rfind(|c: char| !c.is_ascii_digit()).map(|i| &fm_name[i+1..]).unwrap_or(fm_name).parse().unwrap_or(0)`. The current version has no correctness bug but allocates two intermediate strings.

### 3.4 `ssf2_box_name_to_fm` mis-strips when there's a digit anywhere in the prefix
[entity_gen.rs:65-71](src/entity_gen.rs#L65):

```rust
let (prefix_lower, raw_num) = if let Some(pos) = lower.find(|c: char| c.is_ascii_digit()) {
    let num: usize = lower[pos..].parse().unwrap_or(1);
    (&lower[..pos], num)
} else {
    (lower.as_str(), 1usize)
};
```

`find(.. is_ascii_digit)` finds the **first** digit. If SSF2 ever names a box `hit3box5` (it doesn't today, but the function is reusable), this would return prefix=`hit`, num=5, which is wrong. `lower.parse::<usize>()` on `"3box5"` returns Err → unwrap_or(1) → 1. Better: search from the right (`rfind` over digits, then validate that the trailing run is purely numeric).

### 3.5 `extract_attack_objects` confuses GetProperty's result with a string
[abc_parser.rs:1356-1362](src/abc_parser.rs#L1356):

```rust
OP_GETPROPERTY => {
    let mn_idx = read_u30_at(bytecode, &mut i).unwrap_or(0);
    let name = abc.multinames.get(mn_idx as usize).map(|m| m.name.clone()).unwrap_or_default();
    if !stack.is_empty() { stack.pop(); }
    stack.push(StackVal::Str(name));
}
```

`getproperty` should leave the *value* of the property on the stack, but this puts the *property name* there. Downstream code that does `if let StackVal::Str(k) = &chunk[0]` to read object-literal keys (line 1302) won't be fooled — keys come from `pushstring`, not `getproperty` — but anywhere a getproperty value is subsequently used as a numeric operand (e.g. inside an arithmetic expression that contributes to an attack-stat value), the synthetic-name-as-string fallback will quietly stand in. Very low-probability bug in practice (attack-stat values are nearly always literals), but worth a comment if it's intentional, or a `StackVal::Unknown` push if not.

### 3.6 `apply_call_splits` doesn't word-boundary the source method name
[api_mappings.rs:1047](src/api_mappings.rs#L1047):

```rust
let needle = format!(".{}(", method);
```

If `method` is `"updateAttackStats"`, the needle is `.updateAttackStats(`. Substring search will only match where preceded by `.` and followed by `(`, which prevents the `notupdateAttackStats(` false positive. **But** if a future entry adds a shorter method name that's a suffix of a longer one (e.g. `"AttackStats"` and `"updateAttackStats"`), the shorter one would match inside the longer one's text. Not a current bug, but the routing is BTreeMap-ordered (alphabetical), not length-sorted, so ordering issues could surface silently.

### 3.7 `infer_ext_var_types` classifies floats as Int
[api_mappings.rs:1436-1441](src/api_mappings.rs#L1436):

```rust
let kind = match init_lookup.get(name).map(|s| s.trim()) {
    Some("true") | Some("false") => ExtVarType::Bool,
    Some(s) if s.parse::<i64>().is_ok() => ExtVarType::Int,
    Some(s) if s.parse::<f64>().is_ok() => ExtVarType::Int,
    _ => ExtVarType::Object,
};
```

A float literal like `"0.5"` parses as `f64` and gets classified as `ExtVarType::Int`, which then emits `self.makeInt(0)` ([haxe_gen.rs:887](src/haxe_gen.rs#L887)) and lets `wrap_persistent_state` rewrite `self.var++` → `var.inc()`. A float stored through `var.set(0.5)` and then read as a Haxe `Int` will round, lose precision, or fail to compile depending on the FM template. **Repro**: a character with `public var jumpScale:Number = 1.5;` in the Ext-class iinit → emitted as `self.makeInt(0)` then assigned `1.5`.

Fix: separate the two arms — `is_ok::<i64>` → Int; `is_ok::<f64>` → Float (a new variant), with a `makeFloat` factory if FM exposes one; otherwise treat float-init as Object.

### 3.8 `prerender_skewed_frames` `next_id` collides with future bakes from the *next* character
[image_extractor.rs:379](src/image_extractor.rs#L379):

```rust
let mut next_id: u16 = images.keys().max().copied().unwrap_or(0).max(59_999) + 1;
```

`u16` overflow at id 65535 silently wraps to 0 and starts colliding with real shape IDs. SSF2 SWFs can reach the high tens-of-thousands of shape IDs already. A character with ~5000 sheared placements would overflow. The cache deduplicates so this is unlikely to hit; but worth a `checked_add` or a switch to `u32` for the synthetic-id namespace.

### 3.9 `strip_jsonc` doesn't handle backslash inside a string before a `"` correctly
[mappings.rs:227-241](src/mappings.rs#L227): the **first** pass tracks string state with this snippet:

```rust
if in_string {
    decommented.push(c);
    if c == '\\' {
        if let Some(n) = chars.next() { decommented.push(n); }
    } else if c == '"' {
        in_string = false;
    }
    continue;
}
```

This is fine. But the **second** pass (trailing-comma removal, line 248-268) doesn't honour escapes the same way — its `in_string` only checks for `c == '\\'` to skip-pair. If a string contains `"\""` (escaped quote followed by literal quote), the first pass passes it through; the second pass sees `\\` → skip-pair, then `"` again, then `,`. Could miscount string state on edge-case strings with escaped quotes near trailing commas. The JSONC files in this repo don't contain such strings, so this isn't currently a live bug.

### 3.10 `aerial_cap_todo` checks the wrong stat
[haxe_gen.rs:563](src/haxe_gen.rs#L563):

```rust
aerial_cap = fmt(aerial_cap), aerial_cap_todo = todo(s.air_mobility),
```

`aerial_cap` is computed from a derivation that takes `air_mobility_raw` and `aerial_friction`; the `/*TODO*/` comment is gated on `s.air_mobility != 0.0`. If air_mobility is 0 but aerial_friction is nonzero, the derivation could still produce a nonzero `aerial_cap` while the line gets a `/*TODO*/` marker. Conversely, if both are nonzero but produce a zero derivation result, no TODO is emitted. The marker should be `todo(aerial_cap)`.

### 3.11 `read_u30` allows shift overflow on malformed ABC
[abc_parser.rs:259-271](src/abc_parser.rs#L259) and the helper at [abc_parser.rs:1935](src/abc_parser.rs#L1935):

```rust
loop {
    let b = self.read_u8()? as u32;
    result |= (b & 0x7F) << shift;
    shift += 7;
    if b & 0x80 == 0 || shift >= 35 {
        break;
    }
}
```

`shift` can reach 35 (after 5 bytes), so `(b & 0x7F) << 35` is well-defined for `u32` (it's 0-shifts past the high bit), but the resulting `result` will silently truncate values that need more than 32 bits. Harmless for valid ABC, but for fuzzed input the loop runs at most 5 times and exits — no infinite loop, no panic. Document this is intentional (or add a `debug_assert!` so we catch malformed bytecode in tests).

### 3.12 `prerender_skewed_frames` `det.abs() < 1e-9` skips ALL flipped placements with tiny dets
[image_extractor.rs:441](src/image_extractor.rs#L441).

```rust
let det = wa * wd - wb * wc;
if det.abs() < 1e-9 { continue; }
```

The threshold is fine for near-singular matrices; the suspect issue is that `continue` *silently* drops the entire frame's placement. There's no fall-back to the scale+rotation path. For a character whose root MC scale is near-zero (which the codebase warns about elsewhere — "near-zero scales (hidden sprites, e.g. flying)"), an unfortunate composite could trigger this and silently lose a frame's image. Worth either a warning log or a fall-back.

### 3.13 `find_collision_box_base_size` measures `ItemBox` indirectly through `tally.iter().max_by_key`
[sprite_parser.rs:673](src/sprite_parser.rs#L673).

```rust
if let Some((&char_id, _)) = tally.iter().max_by_key(|(_, &count)| count) {
```

`max_by_key` returns the LAST maximum on ties in `tally.iter()` (which iterates a `BTreeMap` in key order). Two different collision-box shapes used equally often → the higher-numbered char_id wins arbitrarily. If a character has both `CollisonBox_6` (id 112) and a custom box shape both used 100 times, the higher-id one is picked, and `measure_box_char` measures the wrong shape, leading to wrong absolute box sizes across the whole character. Mitigate with a deterministic tiebreaker (smallest char_id, or the one matching `collisonbox` naming).

### 3.14 `_v0` slot name collision when `setlocal_0` is used
[decompiler.rs:268-271](src/decompiler.rs#L268):

```rust
let var_name = if *n == 0 {
    "self".to_string()
} else {
    format!("_v{}", n)
};
```

Local 0 is `this` in AVM2. SSF2 frame scripts sometimes do `setlocal_0` (legitimately rebinding `this`-as-local), which the decompiler renders as `self = …`. That's a syntax error in Haxe (`self` is `final` from the surrounding template). The next condition guards against `setlocal_0` of `Expr::This` to itself but not for an arbitrary expression. **Repro**: a frame script that does any kind of `setlocal_0` produces uncompilable Haxe.

### 3.15 `fix_intangibility_pairs` pairs across animations if frame numbers happen to align
[api_mappings.rs:937-963](src/api_mappings.rs#L937):

```rust
calls.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
…
for (i, (pfx, true_frame, true_line, is_true)) in calls.iter().enumerate() {
    if !is_true { continue; }
    if let Some((_, false_frame, false_line, _)) = calls[i+1..].iter()
        .find(|(p, _, _, is_t)| p == pfx && !is_t)
    {
```

The `find` only checks `p == pfx`. Since the sort is by `(prefix, frame)`, finding the "next false in same prefix" is correct *within an animation* but a `true` at the end of the last animation could still match a `false` from a fully-unrelated prefix if the false were considered. Actually the predicate guards `p == pfx`, so OK. **But**: nested duplicate scripts (e.g. someone with two animations of the same prefix that share a frame-script template) would pair across them. Worth a test.

### 3.16 `parse_object_fields` doesn't allow string keys
[api_mappings.rs:1279-1319](src/api_mappings.rs#L1279). The loop accepts identifier-shaped keys only:

```rust
while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
let field_name: String = chars[name_start..i].iter().collect();
```

SSF2's decompiled output uses bare keys so this is fine, but a quoted key like `"foo bar": 1` (which Haxe allows in dynamic objects) would skip the entire field silently. If `apply_call_splits` ever processes hand-edited frame scripts with quoted keys it'd drop fields without warning.

### 3.17 `comment_out_unknown_calls` matches across string boundaries
[api_mappings.rs:1498-1534](src/api_mappings.rs#L1498).

`line.contains(m)` doesn't care if `.SOMETHING(` is inside a string literal. A line like `var msg = "self.bringInFront(x)";` would get matched and rewritten as `// [SSF2-only: bringInFront] var msg = "self.bringInFront(x)";`, breaking valid code. Unlikely in decompiled SSF2 output (string literals rarely contain method-call syntax), but consider being string-aware here.

### 3.18 `extract_xframe_transforms`: `current_ssf2_label = None` after every ShowFrame
[sprite_parser.rs:269-270](src/sprite_parser.rs#L269):

```rust
swf::Tag::ShowFrame => {
    if let (Some(label), Some(m)) = (&current_ssf2_label, &stance_matrix) {
        …
    }
    current_frame += 1;
    current_ssf2_label = None;
}
```

The label is cleared after the first ShowFrame on it. If a single FrameLabel is followed by *multiple* ShowFrames (the label spans several frames), only the first frame's stance matrix gets recorded. For animations with consistent root MC scale across the held frames this is OK, but for animations where the root MC swaps in a new stance matrix at a later frame within the same label, only the first matrix wins (silently). The other matrices would still be used per-frame for in-sub-sprite stuff, just not in the per-animation xform_map. The comment in [sprite_parser.rs:269](src/sprite_parser.rs#L269) suggests the author was aware ("doesn't re-record"), but it's worth verifying it's what we want.

### 3.19 `wrap_persistent_state` assign regex misses the boundary between `;` chains
[api_mappings.rs:1482-1485](src/api_mappings.rs#L1482):

```rust
let assign_re = regex::Regex::new(
    &format!(r"\bself\.{}\s*=\s*(?P<rhs>[^=;\n][^;\n]*?);", regex::escape(name))
).unwrap();
```

The regex is `[^=;\n][^;\n]*?`. The first char excludes `=`, `;`, `\n`. The rest excludes `;` and `\n`. A multi-line RHS (e.g. an inline function literal that the decompiler produced for a closure) would not match — the closure body crosses newlines, and the regex would fail to find a closing `;`, so the assignment isn't rewritten, leaving a dangling `self.X = function() {...}` that `wrap_persistent_state` later turns into `X.get() = function() {...}` via the read pass. **Repro**: a character with an Ext-class `public var onHit:Function = function() {…};` would assign a multi-line closure. Today's decompiler emits closures over multiple lines (`render_closure`), so this is at least theoretically reachable.

### 3.20 `looks_like_char_name` rejects numeric leading char
[abc_parser.rs:2364-2368](src/abc_parser.rs#L2364):

```rust
fn looks_like_char_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false)
        && s.len() >= 3 && s.len() <= 24
}
```

Strictly correct for SSF2's lowercase character names, but if a future entry has a numeric prefix (`8bitmario` etc.) the costume map would lose it. Probably intentional. Just flagging.

### 3.21 `decompile_method` doesn't reset `visited` between calls
[decompiler.rs:1043](src/decompiler.rs#L1043) creates a new `StructuredDecoder` per `decompile_method` call, so this isn't a bug — but the field is named `visited` (a set of block start offsets) and the docstring doesn't say it's intentionally one-shot per method body. Worth a comment to prevent future misuse.

### 3.22 `extract_costume_data_from_apply_palette` populates only the `colors` field
[abc_parser.rs:2257-2262](src/abc_parser.rs#L2257):

```rust
costumes.push(CostumeData {
    name: format!("Alt {}", idx + 1),
    colors: vec![0xFF000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)],
    replacements: vec![],
});
```

Each costume gets exactly **one** color — the multiplier color. There's no per-slot palette mapping like the live `scan_all_costume_methods` path produces. Since this function is dead today (per §2.2), this is academic; but if it's ever re-enabled it'll emit broken palettes (`colors.len() != replacements.len()`, but `replacements: vec![]`).

### 3.23 `OP_CONSTRUCTPROP` in `extract_attack_objects` doesn't push the right shape
[abc_parser.rs:1394-1399](src/abc_parser.rs#L1394):

```rust
OP_CONSTRUCTPROP => {
    read_u30_at(bytecode, &mut i);
    let argc = read_u30_at(bytecode, &mut i).unwrap_or(0) as usize;
    let drain = stack.len().min(argc + 1);
    stack.drain(stack.len() - drain..);
    stack.push(StackVal::Unknown);
}
```

`constructprop` pops argc args **and** the receiver (`object`), then pushes the constructed instance. The current code pops argc+1 (correct) and pushes Unknown (correct). But the AVM2 spec is `constructprop` pops object then args; the order on the stack is `object, arg0, arg1, …, argN`. Code pops the top `argc+1` items — fine. Behaviour is right by coincidence, but a comment would help.

### 3.24 `decode_costume_objects` getproperty(static) preserves `top` for `looks_like_char_name`
[abc_parser.rs:2403-2413](src/abc_parser.rs#L2403):

```rust
} else {
    // Static name property access
    let top = pop!();
    if let Some(name) = static_name {
        if looks_like_char_name(&name) {
            current_char = Some(name);
        }
        stack.push(top); // preserve for chained calls
    } else {
        stack.push(V::Null);
    }
}
```

For a static getproperty, this pops the receiver and pushes it back unchanged, which models AS3's behaviour incorrectly (getproperty replaces receiver with the property value). This presumably works because most static getproperties on SSF2 misc.ssf are on objects whose value we don't care about. But if a chained call pattern `arr.field.method()` ever reaches here, the chained `method`'s receiver will be the original `arr` instead of `arr.field`. Probably benign because of the simple structure of `getCostumeData`.

### 3.25 SSF decompress: implicit conversion of `header_size: u32` to `usize`
[ssf.rs:34](src/ssf.rs#L34):

```rust
let header_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
```

On 32-bit targets `usize == u32`, fine. On 64-bit `as usize` widens. The subsequent check `if compressed_start > data.len()` guards against overflow. But there's a subtle integer-overflow possibility on 32-bit: `compressed_start = 8 + header_size` could wrap if header_size is `u32::MAX - 7`. Use `checked_add` to be safe.

### 3.26 `parse_sounds` uses uncontrolled `swf[pos+i]` slice reads
[sound_extractor.rs:84-94](src/sound_extractor.rs#L84):

```rust
if length >= 7 {
    let char_id     = u16::from_le_bytes([swf[pos], swf[pos+1]]);
    let flags       = swf[pos+2];
    …
    let sample_count = u32::from_le_bytes([
        swf[pos+3], swf[pos+4], swf[pos+5], swf[pos+6]
    ]);
    …
    let data = swf[pos+7..tag_end].to_vec();
```

If `tag_end > swf.len()`, the earlier `if tag_end > swf.len() { break; }` catches it. If `length < 7` but `>= 0`, we skip. If `length == 0` and somehow the tag header says we have a sound, none of these indexing operations happen. So bounds are OK in normal flow, but `if length >= 7` doesn't guarantee `pos+6 < swf.len()` because `pos + length` was already validated to be `<= swf.len()`, and length >= 7 implies pos+6 < tag_end ≤ swf.len(). OK in principle; the indexing-without-explicit-check style is fragile to small refactors. Replacing with `.get()` reads would be safer.

### 3.27 `sub_anim_splits` (jab) emits zero-length sub-animations on duplicate labels
[sprite_parser.rs:493-517 `split_jab`](src/sprite_parser.rs#L493):

```rust
for (i, &(start_frame, _label)) in real_labels.iter().enumerate() {
    let end_frame = real_labels.get(i + 1)
        .map(|&(f, _)| f)
        .unwrap_or(total_frames);
    let fm_name = format!("jab{}", i + 1);
    splits.push((fm_name, start_frame, end_frame));
```

If two real labels share a frame (unusual, but possible — say `combo1` and `combo2` both at frame 12), `jab1` gets `(12, 12)` → zero-length slice. Downstream the `frame_count.max(1)` saves the day, but the resulting animation is bogus. Worth either deduping by frame or warning.

### 3.28 `apply_image_fallbacks` and `apply_fallbacks` don't transitively resolve
[image_extractor.rs:1086-1101](src/image_extractor.rs#L1086), [sprite_parser.rs:584-594](src/sprite_parser.rs#L584).

```rust
if let Some(donor_data) = result.get(*donor) {
    to_insert.push((missing.to_string(), donor_data.clone()));
}
```

If donor itself is also missing (e.g. `swim → fall`, but the character has no `fall` either), the fallback is skipped silently. The table lists `tumble → fall`, `fall_special → ???` etc. — a chain. Today's table just falls back one hop. If `fall` is missing, `swim`/`tumble`/`wall_stick` all silently miss out.

### 3.29 `extract_xframe_name` truncates on length 40
[abc_parser.rs:617](src/abc_parser.rs#L617):

```rust
if !s.is_empty() && s.len() < 40 && s.chars().all(|c| c.is_alphanumeric() || c == '_') {
    return Some(s.clone());
}
```

`s.len()` counts bytes; if a future xframe label is exactly 40 ASCII chars long (rare but possible), it gets rejected. If it's non-ASCII (which the alphanumeric check would already reject), the byte/char counts differ. Probably fine; flag for completeness.

### 3.30 `entity_gen` UUID generation can collide on duplicate instance names
[entity_gen.rs:603](src/entity_gen.rs#L603):

```rust
let sym_id = uuid(char_id, &format!("sym_box_{}_{}_{}", anim_name, inst_name, start_frame));
```

If an animation has *two* boxes with the same instance name (SSF2 sometimes uses `attackBox`, `attackBox` (no suffix) on different depths), they'd map to the same UUID seed and collide. The display-list walk uses depth as the key, so it's not possible for two boxes to share a depth — but if two boxes share an instance name across depths (e.g. duplicated `attackBox` symbols), the higher-depth one wins the `instance_name` slot and the lower-depth one is overwritten in `display_list`. Verify by inspection that SSF2 never uses duplicate instance names within one frame.

---

## Top picks (highest leverage, all sections)

1. **Parse the SWF once per character (§1.1 + §1.2)** — easy refactor, ≈3–10× faster on projectile-heavy characters, cleans up the 11 sites that currently re-parse. Single biggest correctness-preserving perf win in the codebase.
2. **Delete the dead `build_*_map` + `load_api_methods_json` block in api_mappings.rs (§2.1)** — ~350 lines of confusing legacy code that looks load-bearing but is not. Immediate clarity improvement; reduces the surface area for future contributors to mis-read.
3. **Unify the four AVM2 stack simulators in abc_parser.rs (§1.3 + §2.2)** — together with §2.2's dead-code cleanup, this collapses ≈500 lines of duplicated `match op { … }` into one shared scanner. Also eliminates the per-callsite drift bugs (e.g. §3.5's `getproperty` mishandling that exists in some sims but not others).
4. **Fix `infer_ext_var_types` floats-as-Int bug (§3.7)** — concrete correctness issue: any character with a non-integer Ext-class numeric var emits a broken `self.makeInt(0)` wrapper. Trivial fix, real impact.
5. **Cache regexes in `wrap_persistent_state` (§1.7)** — easy win because it currently runs hundreds of times per character and recompiles 2–4 regexes per ext_var per call. Even a per-character `OnceLock` would erase most of the cost.
