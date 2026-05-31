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

(As of HEAD `7729ecda`.)

| crate         | version  | licence                                    |
|---------------|----------|--------------------------------------------|
| anyhow        | 1.0      | MIT OR Apache-2.0                          |
| byteorder     | 1.5      | Unlicense OR MIT                           |
| bytes         | 1.11     | MIT                                        |
| clap          | 4.6      | MIT OR Apache-2.0                          |
| **colored**   | 2.2      | **MPL-2.0** — file-level copyleft; no modification needed for our use, but the source remains under MPL-2.0 |
| **encoding_rs** | 0.8    | (Apache-2.0 OR MIT) **AND BSD-3-Clause** — combined |
| env_logger    | 0.11     | MIT OR Apache-2.0                          |
| fasteval      | 0.2      | MIT                                        |
| flate2        | 1.1      | MIT OR Apache-2.0                          |
| image         | 0.25     | MIT OR Apache-2.0                          |
| indicatif     | 0.17     | MIT                                        |
| log           | 0.4      | MIT OR Apache-2.0                          |
| regex         | 1        | MIT OR Apache-2.0                          |
| serde         | 1.0      | MIT OR Apache-2.0                          |
| serde_json    | 1.0      | MIT OR Apache-2.0                          |
| sha1          | 0.10     | MIT OR Apache-2.0                          |
| sha2          | 0.10     | MIT OR Apache-2.0                          |
| **swf**       | 0.2      | MIT OR Apache-2.0 (Ruffle)                 |
| tempfile      | 3        | MIT OR Apache-2.0                          |
| tokio         | 1.51     | MIT                                        |

The MPL-2.0 (`colored`) and BSD-3-Clause (`encoding_rs`) licences
require the licence text to travel with redistributions. Both licence
texts are reproduced in full under each crate's published source
repository; for a binary distribution of this converter, this `NOTICE.md`
file together with the named source URLs satisfies the attribution
requirement.

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

## Bundled fonts

The cross-platform GUI (`ssf2-converter-gui`) embeds the **Roboto** font
(Regular / Medium / Bold) under `ssf2-converter-gui/assets/`. Roboto is
© Google, licensed under the **Apache License 2.0**, which permits
redistribution with attribution (satisfied by this notice).

## This project's own licence

This project is licensed under the **MIT License** — see
[`LICENSE`](LICENSE) for the full text. MIT is compatible with every
dependency listed above (all are MIT, MIT/Apache-2.0, or carry only
attribution-style requirements satisfied by this NOTICE).
