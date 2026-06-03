# 💉 Peptide

### the mod development + debugging platform for Fraymakers -- and your bridge from Super Smash Flash 2. ⚔️

**Peptide is a master orchestrator for building Fraymakers mods.** It hooks into
Fraymakers and FrayTools and makes them do what you want -- debug a live match,
quick-launch into a sandbox, export a project hands-free -- and it ships a full Super
Smash
Flash 2 interpreter that rebuilds SSF2 characters into a working Fraymakers starting
point you can keep building on. one app, one binary. 🚀

---

## ✨ what it does

### 🔬 debug live -- drive the real engine
An external sandbox debugger for Fraymakers. Spawn an entity, run any move, and read
back exactly what happens, all scripted from the command line. You can even run
higher-level un-sandboxed hscript against any object live. (careful with that one! 😅)

### 🛟 load anything -- no more mystery crashes
Peptide keeps Fraymakers from falling over when it loads broken or half-finished
files, and tells you *why* a file is broken instead of just dying. iterate without
fear.

### ⚡ quick-launch -- straight into a match
Boot directly into a sandbox-mode match from the command line, about 10x faster than
clicking through menus. perfect for a tight test loop.

### 🛠️ drive FrayTools -- export hands-free
Hook your local FrayTools and tell it to publish a project to `.fra`. no manual
clicking, no babysitting the export.

### 🔄 convert from SSF2 -- a whole character, one command
Point Peptide at a single `.ssf` file and it rebuilds the fighter as a ready-to-open
FrayTools project -- sprites, animations, collision boxes, costumes, sounds,
projectiles, effects, menu art, and gameplay logic -- then you keep working on it with
Peptide + FrayTools. (WIP, roughly 70% of the way there.)

```bash
peptide convert mario.ssf      # → a Fraymakers character you can keep building 🎉
```

> 💡 there's a whole Flash interpreter in here (built on [Ruffle](https://ruffle.rs)),
> so even logic Peptide doesn't auto-convert still lands as human-readable hscript in
> the entity scripts. nothing's a black box.

---

## 🌟 what the converter brings over

- 🎨 **Pixel-faithful sprites** -- every frame extracted and re-placed (position,
  rotation, scale, flip), plus inline effects like trails, with shears Flash could
  draw but FrayTools can't pre-baked so they still look right.
- 📐 **Sub-pixel collision** -- hit / hurt / touch / grab / ledge / reflect / absorb
  boxes, plus an auto-fitted body diamond, with rotated boxes un-broken via a bounding
  box. validated against the SSF2 source.
- 🚀 **Projectiles** -- ripped out into their own entity files.
- 🧠 **Readable logic** -- a from-scratch ActionScript decompiler reconstructs each
  character's behavior into Haxe, rewritten to the Fraymakers API.
- 🎨 **Every costume** -- all the SSF2 color variants become Fraymakers palettes.
- 🔊 **Sounds included** -- character audio converted to Fraymakers `.wav`.
- ⚡ **30 → 60 fps** -- every timing value doubled in lockstep, so playback speed is
  preserved exactly.
- 🔁 **Deterministic** -- re-running the converter is reproducible; diffs reflect real
  changes, not churn.
- 👥 **Two-in-one fighters** -- SSFs that ship a pair (Zelda + Sheik, Bowser + Giga
  Bowser, Wario + Wario Man) convert into a single shared project.

> 🚧 the converter is a work in progress -- stat scaling still needs tuning, menu art
> comes over but isn't wired up, transforming characters and items aren't implemented
> yet, and it's characters-only for now (stages later!). see
> [`docs/STATUS.md`](docs/STATUS.md) for the live list.

---

## 🚀 quick start

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

## 📚 learn more

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

## 📦 requirements

- **Rust** (stable) to build.
- **`ffmpeg`** on `PATH` for sound conversion (optional -- conversion still succeeds
  without it, just skips audio).

## 📄 licence + disclaimers

Licensed under the **Apache License 2.0** -- see [`LICENSE`](LICENSE). Peptide's own
attribution notice is in [`NOTICE`](NOTICE) (keep it with any redistribution); dependency
attribution is collected in [`NOTICE.md`](NOTICE.md).

> Original SSF2 character data © McLeodGaming; Fraymakers / FrayTools © Team Fray.
> Peptide is for personal mod development against assets you already own -- never
> commit or publish their source, bytecode, or assets. See
> [`NOTICE.md`](NOTICE.md) "Reverse-engineering & copyright boundary".

> 🤖 most of Peptide's code was written with Claude Code (AI) -- but the app itself
> uses **no AI when you run it locally**. AI has no place in creative work; it's fine
> for the dumb, labor-intensive plumbing a nonprofit tool like this needs. keep using
> your brain and your imagination -- that's the part no machine can do. 🙂

> 🛡️ Peptide contains **no copyrighted code** from Fraymakers or Adobe. the SWF
> interpreter is built on [Ruffle](https://ruffle.rs).
