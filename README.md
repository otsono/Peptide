# 🧬 Peptide

### Bring your favorite Super Smash Flash 2 fighters to life in Fraymakers. ⚔️

**Peptide is a one-stop modding toolkit that converts SSF2 characters into
[Fraymakers](https://fraymakers.com) mods -- and lets you test them in
the real engine without leaving your terminal.** One app, one binary, three
superpowers. 🚀

---

## ✨ What it does

### 🔄 Convert -- a whole character, one command
Point Peptide at a single `.ssf` file and it rebuilds the **entire** fighter as a
ready-to-open FrayTools project -- sprites, animations, collision boxes, costumes,
sounds, projectiles, effects, menu art, and decompiled gameplay logic. No manual
ripping, no rebuilding timelines by hand, no copy-pasting stat tables.

```bash
peptide convert mario.ssf      # → a complete Fraymakers character package 🎉
```

### 🧪 Test live -- drive the real engine
Boot Fraymakers, spawn your converted fighter, run any move, and read back exactly
what happens -- all scripted from the command line. Validate a conversion in the
actual game, not just in theory.

### 🛠️ Drive FrayTools -- publish in one click
Hook your local FrayTools over the DevTools protocol to publish a project to `.fra`,
render an entity, or pull box geometry -- automatically.

---

## 🌟 Highlights

- 🎨 **Pixel-faithful sprites** -- every frame extracted and re-placed (position,
  rotation, scale, flip), with shears Flash could draw but FrayTools can't pre-baked
  so they still look right.
- 📐 **Sub-pixel collision** -- hitboxes, hurtboxes, grab / ledge / reflect / absorb
  boxes, plus an auto-fitted body diamond, validated against the SSF2 source.
- 🧠 **Decompiled logic** -- a from-scratch ActionScript decompiler reconstructs each
  character's behavior into readable Haxe, rewritten to the Fraymakers API.
- 🎨 **Every costume** -- all the SSF2 color variants become Fraymakers palettes.
- 🔊 **Sounds included** -- character audio extracted to WAV.
- ⚡ **30 → 60 fps** -- every timing value doubled in lockstep, so playback speed is
  preserved exactly.
- 🔁 **Deterministic** -- re-running the converter is reproducible; diffs reflect real
  changes, not churn.
- 👥 **Two-in-one fighters** -- SSFs that ship a pair (Zelda + Sheik, Bowser + Giga
  Bowser, Wario + Wario Man) convert into a single shared project.

---

## 🚀 Quick start

```bash
cargo build --release        # builds the single `peptide` binary
./build/release/peptide      # launch the app (guided Setup on first run)

# …or straight from the CLI:
./build/release/peptide convert ../ssf2-ssfs/mario.ssf
```

A conversion lands in `./characters/mario/` as a ready-to-open FrayTools project. ✅

> 💡 You bring the assets. Peptide is for personal mod development against SSF2 files
> you already own -- test inputs aren't shipped in this repo.

---

## 📚 Learn more

| Doc | What's inside |
|---|---|
| 📘 [`DEVELOPMENT.md`](DEVELOPMENT.md) | The developer guide -- build, the conversion pipeline, every module, the mapping config, and output layout. **Start here to hack on it.** |
| 🧭 [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) | The authoritative SSF2 / Fraymakers **format** reference (the `.ssf` internals and the `.entity` schema). |
| 🧪 [`TESTING.md`](TESTING.md) | The two validation harnesses, the end-to-end iteration loop, and in-engine validation status. |
| 🎮 [`docs/PEPTIDE_GUIDE.md`](docs/PEPTIDE_GUIDE.md) | Driving the live engine from the command line -- commands, the iteration loop, recipes. |
| 🏗️ [`docs/PEPTIDE_DESIGN.md`](docs/PEPTIDE_DESIGN.md) | Harness internals -- layering, version-resilience, and the roadmap. |
| 📊 [`docs/STATUS.md`](docs/STATUS.md) | Converter coverage and parity status -- plus the live **known-issues list and prioritized TODOs**. |
| 🤝 [`CONTRIBUTING.md`](CONTRIBUTING.md) | The per-change checklist + hot-file → doc-section map. |

---

## 📦 Requirements

- **Rust** (stable) to build.
- **`ffmpeg`** on `PATH` for sound conversion (optional -- conversion still succeeds
  without it, just skips audio).

## 📄 Licence

Licensed under the **MIT License** -- see [`LICENSE`](LICENSE). Dependency attribution
is collected in [`NOTICE.md`](NOTICE.md).

> Original SSF2 character data © McLeodGaming; Fraymakers / FrayTools © Team Fray.
> Peptide is for personal mod development against assets you already own -- never
> commit or publish their source, bytecode, or assets. See
> [`NOTICE.md`](NOTICE.md) "Reverse-engineering & copyright boundary".
