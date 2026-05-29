# SSF2 → Fraymakers Converter — cross-platform GUI

A pure-Rust [egui/eframe](https://github.com/emilk/egui) desktop app that runs
on **Windows, macOS, and Linux**. It mirrors the macOS SwiftUI app
(`../SSF2ConverterApp/`) but ships as a single self-contained executable with
no runtime dependencies (no WebView2, no Electron, no bundled browser).

## What it does

- Drag-and-drop (or pick) an `.ssf` file → converts it by invoking the
  `ssf2_converter` binary that sits next to it.
- Pickers for the output folder, `misc.ssf` (palettes), and your FrayTools
  executable — all persisted across launches.
- Optional **"Publish into Fraymakers?"** prompt on first run: if Fraymakers is
  installed, offers to auto-add the `custom/<Character>` folder to each
  converted character's FrayTools publish settings (Yes / Not now / Don't ask
  again).
- **Export in FrayTools** button: drives your FrayTools install (via the Node
  harness in `../tools/fraytools-harness/`) to publish the game-ready `.fra`
  into every configured output folder.
- Surfaces the conversion log (unknown / SSF2-only calls) in a window.

## Platform-specific paths (handled automatically)

| | Fraymakers custom folder | FrayTools default | node lookup |
|---|---|---|---|
| **Windows** | `%APPDATA%\Steam\steamapps\common\Fraymakers\custom\<Char>` | `%LOCALAPPDATA%\Programs\FrayTools\FrayTools.exe` | `%ProgramFiles%\nodejs`, then PATH |
| **macOS** | `~/Library/Application Support/Steam/steamapps/common/Fraymakers/custom/<Char>` | `/Applications/FrayTools.app` (→ inner binary) | Homebrew/usr, then PATH |
| **Linux** | `~/.steam/steam/steamapps/common/Fraymakers/custom/<Char>` | (pick manually) | usr, then PATH |

FrayTools `publishFolders` paths are always written **relative** to the
project dir with forward slashes — FrayTools ignores absolute paths.

## Build & run

The GUI is a workspace member, so it shares the repo's `target/` dir with the
`ssf2_converter` binary — which means it finds the converter right next to its
own executable.

```sh
# from the repo root — builds BOTH the converter and the GUI
cargo build --release -p ssf2_converter --bin ssf2_converter
cargo build --release -p ssf2-converter-gui

# run
./target/release/ssf2-converter-gui          # macOS / Linux
.\target\release\ssf2-converter-gui.exe       # Windows
```

For distribution, ship `ssf2-converter-gui[.exe]` and `ssf2_converter[.exe]`
in the same folder.

### Windows notes

- Build with the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
  or cross-compile from another OS with the appropriate target + linker.
- The `windows_subsystem = "windows"` attribute means release builds run with
  no console window.
- The **Export in FrayTools** feature additionally needs Node.js installed and
  the `tools/fraytools-harness/` `node_modules` present
  (`cd tools/fraytools-harness && npm install`). Conversion itself does not
  need Node.

## Relationship to the SwiftUI app

`../SSF2ConverterApp/` remains the macOS-native build (SwiftUI can't target
Windows). This egui app is the cross-platform equivalent and is the path
forward for Windows/Linux users. Both shell out to the same `ssf2_converter`
binary and the same FrayTools harness, so behavior is identical.
