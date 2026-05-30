# Fraymakers match-launch harness

A CLI-driven programmatic link into a running **Fraymakers** that launches a
match (chosen character / stage / assist) directly into the engine, for fast
iteration testing of converted SSF2 characters — no menu navigation, no input
injection.

## IP boundary

This tool contains **no Fraymakers code, assets, bytecode, or strings**. It
only (a) reads the user's *local* Fraymakers bytecode at runtime, (b) writes a
patched copy back into the user's *local* install dir, and (c) speaks a simple
line protocol over a loopback TCP socket. Fraymakers is McLeodGaming
proprietary software; its bytecode (`hlboot-sdl.dat`), any patched output
(`_conn.dat`), and any `.fra` packages stay on the user's machine and are
**never committed** (see `.gitignore`).

## How it works

`fray_patch` parses the engine's HashLink bytecode (via the `hlbc` crate),
injects a small per-frame block into `fraymakers.Main.update`, and writes a
patched `_conn.dat`. When run, the patched engine:

1. waits for content load (the title screen's "press any button" state),
2. dials a loopback TCP socket back to `frayremote` (auth handshake first), and
3. on receiving `s <char> <stage> <assist>`, builds a real `TrainingMode` and
   calls the engine's own offline match-start flow (gates transition + menu
   teardown), so the match renders exactly as it would from the menus.

Content ids accept short names: a bare `commandervideo` is resolved by
searching the engine's loaded content registry by id (type-segregated:
characters vs. stages), or you can pass a full `namespace::package.id`.

## Bins

- `fray_patch <in.dat> <out.dat> connect <port> <token>` — patch the bytecode.
  Also has read-only inspection modes: `dis <findex>`, `typefields <type>`,
  `fnsof <type>`, `fninfo <findex>`, `callers <findex>`, `strgrep <s>`,
  `whoref <s>`, `inspect`.
- `frayremote serve|send --port <p> --token <t> ["<cmd>"]` — loopback bridge.

## Quick start

```
./run.sh "s commandervideo thespire commandervideoassist" 20
```

`run.sh` is self-contained: it writes `steam_appid.txt`, builds the bins,
patches the bytecode into the install dir, launches the engine, and bridges the
command — cleaning up `_conn.dat` afterward. Override the install path with
`FRAY_DIR=...`.

## Status / known issues

See `memory/project_fraymakers-match-launch.md` for the full RE map and the
current open items (non-consuming socket read worked around with a one-shot
launch guard; assist content-type validation; `q` live-match query).
