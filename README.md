# RustEditor

A Rust desktop text and code editor. Features are separated into a Cargo workspace, with extensions provided through dynamically loaded libraries.

Current status: **M0 accepted with maintainer-approved evidence exceptions on 2026-11-03; M1 not started**. The native desktop, editing core, C ABI/SDK, dynamic plugin host, text-tools extension, and development tooling are runnable. Microsoft Pinyin, native 100%/150%/200% scaling and input-to-present tests are passed by maintainer attestation; their original records were not retained. Detailed native/display results are not independently reproducible from that confirmation. Known font/layout limits remain documented.

The [November 3 sign-off and revalidation](docs/acceptance/M0_2026-11-03.md) separates that acceptance decision from archived evidence: 42 local Windows Rust tests and 4 Python tests passed, native 125% multiline Notepad clipboard/shortcut/close checks passed, and 50 release idle working-set samples peaked at 149.7070 MiB. [Main CI at `c65c271`](https://github.com/ananwanan/RustEditor/actions/runs/41481683607) passed on Windows and Ubuntu after the LF checkout fix. The [historical M0 record](docs/acceptance/M0.md) is preserved. M1 is unblocked; no later milestone features are claimed.

- [Architecture, technology choices, and workspace](docs/ARCHITECTURE.md)
- [Dynamic library plugin protocol](docs/PLUGIN_ABI.md)
- [Development roadmap and acceptance criteria](docs/ROADMAP.md)

Initial target: Windows 10/11 x64, followed by Linux and macOS. The application runs as a standalone desktop program without a WebView. It independently implements a similar editing experience and does not promise binary compatibility with Notepad++ plugins.

## Run the M0 preview

Install Rust using [rustup](https://rustup.rs/). The project follows the **Rust stable channel** rather than pinning a specific compiler version. Use the following `rust-toolchain.toml` configuration to select stable Rust with rustfmt and clippy:

```toml
[toolchain]
channel = "stable"
profile = "minimal"
components = ["rustfmt", "clippy"]
```

Run `rustup update stable` to update the installed stable toolchain. On Windows, use the x64 MSVC toolchain and Visual Studio C++ build tools.

From the repository root:

```sh
cargo xtask stage
cargo run --locked -p rusteditor-desktop
```

Select text and run **Plugins → Uppercase selection**, **Run command** in the Extensions panel, or **Ctrl+Shift+U**. The example extension is a separately built native library, not linked into the desktop. Its transaction preserves Chinese/emoji and supports Unicode case expansion such as `straße` → `STRASSE`. Ctrl+Z undoes; Ctrl+Y or Ctrl+Shift+Z redoes. Clipboard shortcuts are handled by the native GUI integration.

The editor is currently a **temporary scratch buffer**: no file opening/saving, tabs, encoding conversion, recovery, or large-file mode yet. Modified scratch text requires a discard confirmation before closing. Documents are limited to 4 MiB; undo retains at most 256 transactions and 8 MiB of text deltas. The core-driven editor-view uses extended grapheme boundaries for horizontal movement, selections, hit testing, and deletion. IME preedit remains a presentation overlay until one commit transaction; the acceptance record distinguishes archived native observations from maintainer-attested passes.

Chinese glyphs use an installed system font (Microsoft YaHei on Windows, Noto Sans CJK on Linux, PingFang on macOS). Set `RUSTEDITOR_FONT` to an outline TTF/TTC/OTF path if detection fails. Emoji fallback tries Segoe UI Emoji on Windows and Noto Emoji on Linux; `RUSTEDITOR_EMOJI_FONT` selects an explicit font. Fonts are size-bounded, parsed, and checked for representative outline glyphs before registration; invalid or bitmap-only fonts produce diagnostics and fall back safely. No additional font files are bundled. Emoji render as monochrome glyphs; color and ZWJ ligature shaping are not implemented. Complete platform/font coverage remains follow-up validation beyond the documented M0 prototype limits.

```sh
cargo run --locked -p rusteditor-desktop -- --no-plugins
cargo run --locked -p rusteditor-desktop -- --no-plugins --empty
cargo run --locked -p rusteditor-desktop -- --plugin-dir /absolute/trusted/plugins
```

Use `--empty` to start an empty scratch buffer for GUI measurements. It cannot be combined with the headless `--smoke-test`, which uses a fixed test fixture. The [Windows performance protocol](tests/perf/README.md) provides a real-process memory harness and deterministic UTF-8 workloads. Window discovery timing is diagnostic; editable startup and input-to-present latency require separate presentation evidence.

The experimental plugin SDK targets the current version. Older SDK compatibility is not required; rebuild plugins when the SDK/ABI changes. Version, table-size, capability, and lifecycle checks remain mandatory.

Plugins load at startup from `plugins` next to the executable, or an explicit `--plugin-dir`. Each plugin needs its own subdirectory, platform library, and `plugin.toml`; `cargo xtask stage` creates this layout in the debug output directory. Native plugins have full process privileges: only install code you trust. Close the editor before staging or updating a DLL; changes take effect after restart.

## Validate

```sh
cargo xtask verify
```

This runs formatting, clippy, workspace tests, builds/stages text-tools, and checks real-plugin and no-plugin startup without requiring a display. Tests compile separate ABI/lifecycle fault libraries and run deliberate native aborts only in subprocesses. It does **not** perform Windows IME, DPI, clipboard, or visual acceptance. The [manual checklist](docs/acceptance/M0.md#windows-manual-gate) and [current sign-off](docs/acceptance/M0_2026-11-03.md) track those separately.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI runs the same `cargo xtask verify` on Ubuntu and Windows for main pushes and pull requests. It does not publish releases.

An optional native Linux regression uses a dedicated X11 display with `Xvfb`, `xdotool`, and `xclip` installed:

```sh
cargo xtask stage
xvfb-run -a python3 tests/ui/x11_smoke.py
xvfb-run -a python3 tests/ui/x11_smoke.py --no-plugins
```

Use `--binary PATH` if Cargo uses a non-default target directory. This exercises native clipboard round trips, plugin shortcuts, grapheme navigation/deletion/cut, and undo/redo. It does not validate Windows IME or DPI. Font overrides above may be passed through the environment; the tests compare text bytes independently of glyph coverage.

## Documentation language

All project documentation must be written and maintained in English, including README files, architecture and design documents, development plans, contributor guides, and future documentation additions.
