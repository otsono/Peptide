# Converter Output Cleanup Report

**Date:** 2026-05-30
**Goal:** Remove stale converter output so future regens start from a clean state.

## Where output lives

- `src/main.rs:20` — CLI `--output` defaults to `./characters`. Single-char runs write to `characters/<char>/`.
- `.gitignore:13` — `characters/` is **gitignored** (0 tracked files). All cleanup is on-disk only; nothing shows in git.
- Converter **inputs** live outside this repo: `~/.openclaw/workspace-main/ssf2-ssfs/*.ssf` (46 inputs).
- Build artifacts: `characters/sandbag/build/{sandbag.fra,*.prefix-bak}` — downstream compile output, **not** converter output. Preserved.

## Root cause of staleness

The converter does **not** wipe a character's output dir before writing, so files from old converter versions linger after the writer logic was renamed. Confirmed renamed paths (old → current):

| Old (stale) | Current | Source of truth |
|---|---|---|
| `library/scripts/Character/` | `library/scripts/<Pascal>/` | `src/haxe_gen.rs:26-27` |
| `library/sounds/*.ogg` + `sounds_manifest.json` | `library/audio/*.wav` | `src/haxe_gen.rs:440` |
| `library/entities/Character.entity` | `library/entities/<Pascal>.entity` | entity_gen |
| `library/entities/menu.entity` | `library/entities/Menu.entity` | entity_gen |
| various renamed/reorg'd `library/audio/*.wav` | new names | sound_extractor |

## Method (rigorous, per-character)

The converter is **deterministic** (verified: mario reverse-diff = 1 file; sandbag clean-slate regen = byte-identical file set). So for each character with an input `.ssf`:

1. Regenerate into a temp dir.
2. Guard: regen must succeed, produce a non-`Character` `Script.hx`, and be ≥40% of current file count (catch catastrophic regen failure).
3. Delete `current − fresh` (files the current converter no longer writes), **protecting `build/`**.
4. Prune empty dirs.

Script: `/tmp/cleanup/clean.sh`; per-char manifests in `/tmp/cleanup/stale_<char>.txt`.

## Results

**44 / 45 character dirs cleaned. 7,297 stale files deleted.**

Stale by category (aggregate):
- `library/sounds/` (old `.ogg` audio): **6,292**
- renamed `library/audio/*.wav` + misc: **532**
- renamed `library/entities/*`: **84**
- `sounds_manifest.json(.meta)`: **45**
- `library/scripts/Character/`: **344**

Corpus-wide verification:
- `library/scripts/Character/` directories remaining: **0** (excluding orphan, see below)
- `library/sounds/` directories remaining: **0**
- Sandbag clean-slate regen: 882 files, **byte-identical** to a fresh temp regen; `build/` preserved.

## Question from prior session — RESOLVED

> Is `Character/Script.hx` supposed to still be written?

**No.** It is a confirmed **rename**, not a writer regression. `src/haxe_gen.rs:26-27` writes scripts to `library/scripts/<Pascal>/` and the comment explicitly notes "(was library/scripts/Character/)". A clean sandbag regen produces only `library/scripts/Sandbag/`. The stale `Character/Script.hx` (and its broken loop) were leftovers from pre-rename runs. No further bug to investigate.

## Open items surfaced (no action taken — need your call)

1. **`characters/deespear/` — orphan (1,901 files, all old-converter, May 26–27). RESOLVED: deleted in full** (user decision). There was **no `deespear.ssf` input** in `ssf2-ssfs/`, so it could not be regenerated or regen-diffed; the entire dir was pre-rename stale output and unrecoverable. Removed. Corpus is now **44 char dirs, all current-converter output**.

2. **Steam install — clean, no action needed.** `~/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/` contains only `mario/` and `sandbag/`, each just `<char>.fra` + `meta.json` (2 files). No raw converter output, nothing stale. (Left untouched per user-data rule regardless.)

3. **Unconverted inputs (informational):** `chibirobo.ssf` and `dedede.ssf` exist in `ssf2-ssfs/` with no output dir — never converted. Not a cleanup concern.

## Recommendation

To prevent recurrence, the converter could wipe (or `library/`-scope clean) each char's output dir at the start of a run. Currently it merges into whatever exists, which is what let these renames accumulate.
