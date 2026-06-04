# notice

this project is licensed under the Apache License 2.0, see [`LICENSE`](LICENSE) for full terms.
Peptide's own attribution notice (the one redistributors must carry, per Apache section 4d)
lives in the [`NOTICE`](NOTICE) file; this `NOTICE.md` covers attribution to dependencies as
required by their respective licenses.

this project depends on the open-source crates below. each one is listed with the licence it's
distributed under. their attribution requirements (MIT, Apache-2.0, BSD-3-Clause, MPL-2.0) are
met by this file when the project is redistributed.

## SWF / Flash parsing

* **`swf` 0.2** -- part of the [Ruffle](https://github.com/ruffle-rs/ruffle) project. MIT OR
  Apache-2.0. the converter's entire Flash-parsing path (decompression, tag walking, ABC block
  extraction) sits on top of this crate. ABC decompilation and SSF-wrapper handling are written
  from scratch in this repo, but the SWF binary parsing itself is `swf`'s.

all Flash decompilation (SWF decompression, bitmap decoding, and ABC (ActionScript bytecode)
parsing) happens **in-process** in this repo's own Rust code (`src/ssf.rs`,
`src/swf_parser.rs`, `src/abc_parser.rs`, `src/decompiler.rs`), built only on the `swf` crate
above. there's no external decompiler dependency and **no GPL code** is statically or
dynamically linked (see `DEVELOPMENT.md` §3).

## all direct Rust dependencies

resolved versions (`Cargo.lock`) as of HEAD `6087830d`, spanning both the `peptide` binary and
the `ssf2_converter` library crate.

| crate         | version  | licence                                    |
|---------------|----------|--------------------------------------------|
| anyhow        | 1.0      | MIT OR Apache-2.0                          |
| byteorder     | 1.5      | Unlicense OR MIT                           |
| bytes         | 1.11     | MIT                                        |
| clap          | 4.6      | MIT OR Apache-2.0                          |
| **colored**   | 2.2      | **MPL-2.0** -- file-level copyleft; no modification needed for our use, but the source remains under MPL-2.0 |
| crossterm     | 0.29     | MIT                                        |
| dirs          | 5.0      | MIT OR Apache-2.0                          |
| **encoding_rs** | 0.8    | (Apache-2.0 OR MIT) **AND BSD-3-Clause** -- combined |
| env_logger    | 0.11     | MIT OR Apache-2.0                          |
| fasteval      | 0.2      | MIT                                        |
| flate2        | 1.1      | MIT OR Apache-2.0                          |
| hlbc          | 0.7      | MIT -- Fraymakers engine bytecode reader/writer |
| image         | 0.25     | MIT OR Apache-2.0                          |
| indicatif     | 0.17     | MIT                                        |
| log           | 0.4      | MIT OR Apache-2.0                          |
| ratatui       | 0.30     | MIT                                        |
| regex         | 1        | MIT OR Apache-2.0                          |
| rfd           | 0.15     | MIT                                        |
| serde         | 1.0      | MIT OR Apache-2.0                          |
| serde_json    | 1.0      | MIT OR Apache-2.0                          |
| sha1          | 0.10     | MIT OR Apache-2.0                          |
| sha2          | 0.10     | MIT OR Apache-2.0                          |
| **swf**       | 0.2      | MIT OR Apache-2.0 (Ruffle)                 |
| tao           | 0.35     | Apache-2.0                                 |
| tempfile      | 3        | MIT OR Apache-2.0                          |
| **tiny-skia** | 0.11     | **BSD-3-Clause** -- vector-shape rasteriser |
| tokio         | 1.51     | MIT                                        |
| toml          | 0.8      | MIT OR Apache-2.0                          |
| tungstenite   | 0.21     | MIT OR Apache-2.0                          |
| wry           | 0.55     | Apache-2.0 OR MIT                          |

the MPL-2.0 (`colored`) and BSD-3-Clause (`encoding_rs`, `tiny-skia`) licences require the
licence text to travel with redistributions. each licence text is reproduced in full under
that crate's published source repository; for a binary distribution, this `NOTICE.md` file
together with the named source URLs satisfies the attribution requirement.

## asset and game data

original Super Smash Flash 2 character data, sprites, sounds, and costumes are © McLeodGaming.
this tool is meant for personal mod development against assets the user already owns; no SSF2
game data ships in this repository (`.gitignore` excludes `*.ssf` / `misc.ssf`).

the Fraymakers asset format and FrayTools editor are © Team Fray /
[Fraymakers](https://fraymakers.com). the output this tool produces is meant for
use with FrayTools to develop custom characters; no Fraymakers engine code or proprietary
tooling is included.

## reverse-engineering & copyright boundary -- NEVER PUBLISH

SSF2 (© McLeodGaming), and Fraymakers / FrayTools (© Team Fray) are **proprietary,
copyrighted software**. this repository reverse-engineers their file formats and behaviour
**for interoperability only**, and it does that by describing observations **in our own
words**.

the following must **never be committed, published, or redistributed** in this repo (or
anywhere), because it's copyrighted third-party material:

- **source code, bytecode, or disassembly** from FrayTools (`app.asar` / bundle JS), the
  Fraymakers engine (`hlboot-sdl.dat` / engine bytecode / Haxe source / `.hx:line` stack
  traces), or SSF2 (AS3 / ABC bytecode).
- **decompiled output of their code** pasted verbatim (it's a derivative work of their
  copyrighted source).
- **their assets or data files** -- `.ssf` / `.swf` / `misc.ssf`, `.fra` packages, sprites,
  sounds, palettes, or any extracted strings.

these are all git-ignored (`*.ssf`, `*.swf`, `characters/`, `tools/**/node_modules`, the
`docs/` scratch folder, etc.) and stay only on the user's machine.

**engine internals -- kept out to respect Team Fray's wishes.** on top of the rule above
(never publish their *material*), Team Fray has asked that the tracked docs not
**explain how to decompile or patch** the Fraymakers engine binary or the FrayTools
editor bundle, and not **name specific non-hscript engine classes, functions, or fields** or
document the engine's internal symbol map (the named move-dispatch / telemetry / content-load
functions, type/field layout, the `CState` integer values, the FrayTools render internals).
to respect Team Fray's wishes, we haven't included that material in the tracked docs (please
keep it that way); it lives only in the user's local, gitignored `docs/` scratch space for
interoperability work. what's
fine to document, and stays, is what Peptide does and how to work on it (the commands, the
patcher architecture and workflow, resolve-by-name, the `doctor` preflight) plus the public
hscript scripting API. this restriction is specific to the Fraymakers engine and FrayTools;
the SSF2 / SWF / ABC *input*-format notes the converter relies on aren't affected. see
[`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) "engine-side knowledge is not in this repo".

what this repo **may** contain: our own Rust/JS/docs, format descriptions and RE notes written
in our own words, symbol *names* cited as facts for interop, and **illustrative** schema
examples we authored (e.g. the `.entity` JSON shapes in `AGENT_CONTEXT.md`), never copied from
a specific copyrighted file. if you add RE notes, paraphrase, don't paste.

## bundled fonts

none. the Peptide UI is a system webview (WKWebView / WebView2 / WebKitGTK) and uses the host's
system fonts, so no fonts are bundled or redistributed.

## this project's own licence

this project is licensed under the **Apache License 2.0**, see [`LICENSE`](LICENSE) for the
full text. Apache-2.0 is compatible with every dependency listed above (all are MIT,
MIT/Apache-2.0, BSD-3-Clause, or file-level MPL-2.0, which carry only attribution-style
requirements that this NOTICE satisfies). redistributors must also carry the [`NOTICE`](NOTICE)
file's contents alongside this attribution.
