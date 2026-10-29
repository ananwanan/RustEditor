# Architecture Plan

Date: 2026-10-28. The M0 prototype implements the desktop/core/view/api/sdk/host/text-tools/xtask path; **acceptance remains incomplete**. Later workspace members below remain plans. Rust and dependency versions are locked for reproducible prototype builds; GUI and ABI acceptance remain provisional until the [M0 gates](acceptance/M0.md) pass. M1 remains blocked by those mandatory gates.

## M0 Implementation Decisions

- Virtual resolver-2 workspace, Stable Rust, edition 2024, shared metadata/lints, and committed Cargo.lock. No project license has been selected; no distributable release is claimed.
- Retain eframe/egui 0.33.3 with the native Glow backend provisionally for M0 validation. Native Windows checks at 125% scale provide partial evidence for Sogou composition commit/cancel, clipboard, plugin commands, no-plugin editing, and scratch close confirmation. They do not establish Microsoft Pinyin behavior or the required native 100%/150%/200% DPI matrix. An attempted Windows Settings launch exposed no controllable window, so the missing IME/DPI cases remain Pending. Final retention also requires actual editable-frame startup and input-to-presentation evidence. WebView/WASM backends are not used.
- A bounded UTF-8 String is sufficient for the M0 transaction/ABI experiment. Ropey, virtualized layout, and large-file capability remain unvalidated. Maximum document size is 4 MiB, with 256 undo entries / 8 MiB of retained text deltas.
- Document is authoritative. The independent editor-view renders an egui galley and routes ordered keyboard, pointer, clipboard, and IME events into core transactions. It retains no editable String or duplicate undo stack. A bounded transient IME overlay retains the source DocumentId/Revision/range without mutating the document; commit is one validated transaction, cancellation preserves the original selection, and stale/late commits are ignored. Failed/oversized paste preserves the original text, selection and history.
- M0 validation supplies the caret line as the native IME exclusion rectangle instead of the full document rectangle, preventing the document's height from displacing candidates. Regression tests cover its geometry after scrolling, cancellation/session tracking that rejects late commits, and the Windows empty-preedit-before-commit event order. A native Sogou check at 125% scale observed candidates remaining near the caret through a titlebar drag. These results support the fixes; the full Microsoft Pinyin candidate-positioning and native DPI matrix remains an acceptance gate.
- Core revisions start at 1 and increase on edits, undo, and redo. ABI selections use directed UTF-8 byte endpoints. The view converts egui scalar positions and snaps user selections/carets to Unicode extended grapheme boundaries using unicode-segmentation 1.13.3 (Unicode UAX #29). Plain left/right, Shift selection and Backspace/Delete treat combining sequences, ZWJ emoji, flags, keycaps and CRLF as units. Word/vertical and pointer targets are normalized to cluster boundaries. This is navigation behavior, not a claim of complex-script/emoji ligature shaping.
- libloading 0.8.9 loads real cdylibs at startup. Host/SDK meet through the implemented v1.0 C tables in plugin-api; the desktop does not link the plugin crate. Explicit plugin installations and manifests gate loading; Windows uses DLL-directory + System32 search flags, Unix uses RTLD_NOW | RTLD_LOCAL.
- The M0 command registry lives in plugin-host. The editor-view crate was extracted to close the scalar-navigation gap before M1; the commands crate remains planned. No empty placeholders were created. Font installation remains desktop assembly code: bounded outline parsing through ab_glyph 0.2.32, CJK and emoji fallbacks, explicit overrides, and diagnostics; additional fonts are not redistributed.
- Rust FFI boundaries convert unwindable panics to fixed status values. Unknown native state or failed shutdown retains the library, host table, and context until process exit. Disabling only removes dispatch; updates require restart.
- The experimental SDK/ABI targets the current version; older SDK compatibility and historical fixtures are not required. ABI safety checks remain in force. Windows native GUI verification, performance budgets, safe file saving, and platform release support are not established by the current automated tests. Ubuntu/Windows DLL/core CI and a Linux X11 window smoke pass; see the acceptance record for their exact scope and font limitations.
- The [Windows performance protocol and memory harness](../tests/perf/README.md) record reproducible workloads and separate resident process memory from presentation metrics. `--no-plugins --empty` starts an actual empty scratch buffer for measurement. A discovered window handle is only a diagnostic; cold/warm startup to an editable presented frame and input-to-present latency remain Pending. The acceptance record contains measured results and environment limits; no Core timer or CPU frame duration substitutes for display evidence.

## Product Scope

Support everyday text, log, and code editing: multiple tabs, line numbers, a status bar, find and replace, syntax highlighting, encoding conversion, session recovery, and plugins. Priorities are reliable Chinese input, safe file saving, startup speed, and responsive editing.

The first usable release includes explicit open/save policies for UTF-8, UTF-8 BOM, UTF-16 LE/BE, and GBK; CRLF/LF line endings; undo/redo; single-file search and replace; basic highlighting; and one dynamic library plugin. Encoding detection provides candidates and cannot guarantee correctness. A full IDE, debugger, plugin marketplace, binary file editing, and Notepad++ DLL compatibility are outside the first release.

## Workspace Responsibilities

These are target directories to create progressively by milestone. The Cargo workspace defines build and code boundaries. Only extensions marked `cdylib` are loaded at runtime; other crates are compiled and linked into the host.

| Path / package | Artifact | Responsibility |
|---|---|---|
| apps/desktop / rusteditor-desktop | bin | Entry point, windows, menus, tabs, shortcuts, and application assembly |
| crates/editor-core / rusteditor-editor-core | lib | Documents, editing transactions, selections, undo/redo, and revisions; independent of the GUI |
| crates/editor-view / rusteditor-editor-view | lib | Text layout, visible-region rendering, hit testing, scrolling, and IME integration |
| crates/file-io / rusteditor-file-io | lib | Encodings, line endings, file monitoring, safe saving, and recovery |
| crates/commands / rusteditor-commands | lib | Command IDs, routing, shortcuts, and enablement conditions |
| crates/search / rusteditor-search | lib | Literal/regex searches, single-file and directory searches, and replacement transactions |
| crates/syntax / rusteditor-syntax | lib | Parsing, incremental highlighting, and folding information |
| crates/settings / rusteditor-settings | lib | Configuration, themes, sessions, and configuration migration |
| crates/plugin-api / rusteditor-plugin-api | lib | Minimal C ABI types and constants; independent of the GUI and editor core |
| crates/plugin-sdk / rusteditor-plugin-sdk | lib | Rust plugin wrappers, boundary validation, and panic conversion |
| crates/plugin-host / rusteditor-plugin-host | lib | Discovery, loading, negotiation, command registration, and lifecycle management |
| plugins/text-tools / rusteditor-text-tools | cdylib | Example extensions such as case conversion and line sorting |
| xtask / xtask | bin | Packaging, plugin staging, acceptance checks, and release scripts |

Dependency direction: desktop → view/core/file-io/commands/search/syntax/settings/plugin-host; view → core; file-io/search/syntax → core; plugin-host → plugin-api + commands/core; plugin-sdk → plugin-api; plugins → plugin-sdk. The core must not depend on the UI or plugins. Dependency cycles are prohibited.

The M0 dependency audit with `cargo metadata --locked --no-deps --format-version 1` confirms eight implemented members: core and plugin-api have no dependencies; view depends on core and egui; SDK depends on plugin-api; host depends on core, plugin-api, and loading/manifest libraries; text-tools depends on SDK. Desktop assembles view/core/host and does not link text-tools. The command registry remains inside plugin-host until its planned extraction. This validation adds no ABI compatibility layer, hot unloading, file I/O, tabs, or other M1 implementation.

Do not turn every crate into a dynamic library. Frequent internal operations involving documents, layout, and input continue to use Rust types. Extension boundaries use a narrow C ABI. Built-in search supplies baseline functionality, while plugins provide replaceable extension commands so startup does not require third-party DLLs.

## Technology Choices and Validation Gates

| Area | Proposed choice | Rationale and required validation |
|---|---|---|
| GUI | egui + eframe | Efficient Rust desktop prototyping; M0 must validate Windows Chinese IME, scaling, clipboard behavior, and shortcuts |
| Text model | ropey | Candidate chunked text model; benchmark large-file editing, indexing, and snapshot costs |
| Editor view | Core-driven grapheme view implemented; virtualization remains planned | Ordered input and a transient IME overlay operate on the core; the bounded full-document galley does not establish large-file capability |
| Highlighting | tree-sitter, introduced incrementally by language | Separate parsing tasks; tag results with document revisions and discard stale results |
| Search | regex + a custom cross-chunk scanning layer | Rope chunk boundaries can split matches; flattening a whole document is restricted to bounded small-file paths |
| Encoding | encoding_rs + dedicated BOM/UTF-16 handling | Validate legacy encodings such as GBK separately from UTF-16; users must choose how to handle unencodable characters |
| Loader | libloading | Uses platform loaders; the host and plugin remain responsible for safe loading |
| Configuration | serde + TOML | Human-readable configuration with explicit schema versions and migrations |

The GUI choice is provisional. If IME, accessibility, or layout requirements fail in M0, reassess Slint, native controls, or other toolkits before implementing editor features. Do not label an untested dependency version as the latest supported version. Commit Cargo.lock during implementation and record the MSRV, licenses, and target platforms.

## Core Data and Threading

- Identify documents and versions with DocumentId and monotonically increasing Revision values. Keep text, selections, encoding, and line-ending policies separate. Track modified state against the last saved revision.
- Apply edits as transactions and store undo/redo deltas. Distinguish bytes, Unicode scalar values, graphemes, and screen columns. The ABI uses UTF-8 byte offsets at character boundaries; UI cursor movement follows graphemes, and layout determines display width.
- The UI thread exclusively owns mutable editing state. Background tasks handle disk reads, parsing, searches, saves, and recovery snapshots. Tasks must support cancellation, use bounded queues, and validate document revisions before applying results.
- Saving captures an immutable snapshot. Completion marks only that revision as saved; subsequent edits remain unsaved.
- Load small files fully. Use a separate restricted mode for large files, with budgets for highlighting, long-line layout, and undo history. Include GBK/UTF-16 decoding and encoding maps in memory budgets.

## File Reliability

By default, save through a temporary file in the same directory, write and flush it, then synchronize and replace according to platform capabilities. Preserve original permissions and handle read-only files, full disks, file locks, and external modification conflicts. Use a validated replacement strategy on Windows; do not promise identical power-loss atomicity across file systems. A failed save must retain modified state, and external changes must not be overwritten silently.

Keep recovery files separate from original files, with version and integrity metadata. Prompt to restore or discard them at startup. Closing a modified tab must offer Save, Discard, and Cancel. Automatic plugin loading from untrusted directories is disabled by default.

## Development and Distribution

M0 creates a virtual workspace with `resolver = "2"` and shared edition, version, and lint settings. Freeze the edition and MSRV after toolchain validation. Core unit tests run without a GUI; host integration tests load real dynamic libraries. First configure fmt, clippy, and tests for main pushes and pull requests, then establish Windows/Linux/macOS build matrices. Do not add automatic releases in advance.

The initial distribution is a Windows portable package with separate program and user-data directories. Package plugins separately by platform, architecture, and ABI; do not reuse binaries across platforms. Review licenses for fonts, icons, grammars, and dependencies before release. Choose the project license separately and do not reuse Notepad++ assets or trademarks directly.

## References

- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Rust linkage](https://doc.rust-lang.org/reference/linkage.html)
- [Rust FFI](https://doc.rust-lang.org/nomicon/ffi.html)
- [egui Galley](https://docs.rs/egui/0.33.3/egui/struct.Galley.html)
- [Unicode grapheme segmentation](https://docs.rs/unicode-segmentation/1.13.3/unicode_segmentation/struct.GraphemeCursor.html)
- [libloading](https://docs.rs/libloading/latest/libloading/struct.Library.html)

These references document tool capabilities and boundary requirements. The technology choices, directory structure, and milestones above are project design proposals.
