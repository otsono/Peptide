# Notice

This project is licensed under the MIT License — see [`LICENSE`](LICENSE)
for full terms. This `NOTICE.md` covers attribution to dependencies as
required by their respective licenses.

This project depends on the following open-source crates. Each is
listed here with the licence under which it is distributed. Their
attribution requirements (MIT, Apache-2.0, BSD-3-Clause, MPL-2.0) are
met by this file when this project is redistributed.

## SWF / Flash parsing

* **`swf` 0.2** — part of the [Ruffle](https://github.com/ruffle-rs/ruffle)
  project. MIT OR Apache-2.0. The converter's entire Flash-parsing
  path (decompression, tag walking, ABC block extraction) is built on
  top of this crate. ABC decompilation and SSF-wrapper handling are
  written from scratch in this repo, but the SWF binary parsing
  itself is `swf`'s.

All Flash decompilation — SWF decompression, bitmap decoding, and ABC
(ActionScript bytecode) parsing — happens **in-process** in this repo's
own Rust code (`src/ssf.rs`, `src/swf_parser.rs`, `src/abc_parser.rs`,
`src/decompiler.rs`), built only on the `swf` crate above. There is no
external decompiler dependency and **no GPL code** is statically or
dynamically linked (see `DEVELOPMENT.md` §3).

## All direct Rust dependencies

Resolved versions (`Cargo.lock`) as of HEAD `6087830d`, spanning both the
`peptide` binary and the `ssf2_converter` library crate.

| crate         | version  | licence                                    |
|---------------|----------|--------------------------------------------|
| anyhow        | 1.0      | MIT OR Apache-2.0                          |
| byteorder     | 1.5      | Unlicense OR MIT                           |
| bytes         | 1.11     | MIT                                        |
| clap          | 4.6      | MIT OR Apache-2.0                          |
| **colored**   | 2.2      | **MPL-2.0** — file-level copyleft; no modification needed for our use, but the source remains under MPL-2.0 |
| crossterm     | 0.29     | MIT                                        |
| dirs          | 5.0      | MIT OR Apache-2.0                          |
| **encoding_rs** | 0.8    | (Apache-2.0 OR MIT) **AND BSD-3-Clause** — combined |
| env_logger    | 0.11     | MIT OR Apache-2.0                          |
| fasteval      | 0.2      | MIT                                        |
| flate2        | 1.1      | MIT OR Apache-2.0                          |
| hlbc          | 0.7      | MIT — HashLink bytecode reader/writer (the engine patcher's RE backbone) |
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
| **tiny-skia** | 0.11     | **BSD-3-Clause** — vector-shape rasteriser |
| tokio         | 1.51     | MIT                                        |
| toml          | 0.8      | MIT OR Apache-2.0                          |
| tungstenite   | 0.21     | MIT OR Apache-2.0                          |
| wry           | 0.55     | Apache-2.0 OR MIT                          |

The MPL-2.0 (`colored`) and BSD-3-Clause (`encoding_rs`, `tiny-skia`) licences
require the licence text to travel with redistributions. Each licence text is
reproduced in full under that crate's published source repository; for a binary
distribution, this `NOTICE.md` file together with the named source URLs satisfies
the attribution requirement.

## Asset and game data

Original Super Smash Flash 2 character data, sprites, sounds, and
costumes are © McLeodGaming. This tool is intended for personal mod
development against assets the user already owns; no SSF2 game data
is shipped in this repository (`.gitignore` excludes `*.ssf` /
`misc.ssf`).

Fraymakers asset format and FrayTools editor are © Fraymakers /
[Fraymakers](https://www.fraymakersthegame.com/). The output produced
by this tool is intended for use with FrayTools to develop custom
characters; no Fraymakers engine code or proprietary tooling is
included.

## Reverse-engineering & copyright boundary — NEVER PUBLISH

SSF2 (© McLeodGaming), and Fraymakers / FrayTools (© Fraymakers) are
**proprietary, copyrighted software**. This repository reverse-engineers
their file formats and behaviour **for interoperability only**, and it does
so by describing observations **in our own words**.

The following must **never be committed, published, or redistributed** in
this repo (or anywhere), because they are copyrighted third-party material:

- **Source code, bytecode, or disassembly** from FrayTools (`app.asar` /
  bundle JS), the Fraymakers engine (`hlboot-sdl.dat` / HashLink bytecode /
  Haxe source / `.hx:line` stack traces), or SSF2 (AS3 / ABC bytecode).
- **Decompiled output of their code** pasted verbatim (it is a derivative work
  of their copyrighted source).
- **Their assets or data files** — `.ssf` / `.swf` / `misc.ssf`, `.fra`
  packages, sprites, sounds, palettes, or any extracted strings.

These are all git-ignored (`*.ssf`, `*.swf`, `characters/`, `tools/**/node_modules`,
the `docs/` scratch folder, etc.) and stay only on the user's machine.

What this repo **may** contain: our own Rust/JS/docs, format descriptions and
RE notes written in our own words, symbol *names* cited as facts for interop,
and **illustrative** schema examples we authored (e.g. the `.entity` JSON
shapes in `AGENT_CONTEXT.md`) — never copied from a specific copyrighted file.
If you add RE notes, paraphrase; do not paste.

## Bundled fonts

None. The Peptide UI is a system webview (WKWebView / WebView2 / WebKitGTK) and
uses the host's system fonts — no fonts are bundled or redistributed. (The
earlier egui desktop GUI, which embedded the Roboto font, has been removed.)

## This project's own licence

This project is licensed under the **MIT License** — see
[`LICENSE`](LICENSE) for the full text. MIT is compatible with every
dependency listed above (all are MIT, MIT/Apache-2.0, or carry only
attribution-style requirements satisfied by this NOTICE).
