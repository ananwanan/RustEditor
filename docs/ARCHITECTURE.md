# Architecture Plan

Date: 2026-10-28. The M0 prototype now implements the desktop/core/api/sdk/host/text-tools/xtask path. Later workspace members below remain plans. Rust and dependency versions are locked for reproducible prototype builds; GUI and ABI acceptance remain provisional until the [M0 gates](acceptance/M0.md) pass.

## M0 Implementation Decisions

- Virtual resolver-2 workspace, Rust 1.88.0, edition 2024, shared metadata/lints, and committed Cargo.lock. No project license has been selected; no distributable release is claimed.
- eframe/egui 0.33.3 with the native Glow backend; WebView/WASM backends are not used. This is a reproducible MSRV-compatible candidate, not a claim to use the newest release. Windows IME and DPI decide whether to retain it.
- A bounded UTF-8 String is sufficient for the M0 transaction/ABI experiment. Ropey, virtualized layout, and large-file capability remain unvalidated. Maximum document size is 4 MiB, with 256 undo entries / 8 MiB of retained text deltas.
- Document is authoritative. The TextBuffer adapter creates a draft only on a mutation during a widget call, validates it, commits one minimal core transaction at the end, then drops it. Failed/oversized paste rolls back the whole draft. TextEdit's independent undo snapshots are cleared each frame; shortcuts use core history. Drafts are synchronous transaction data, not a second persistent document model.
- Core revisions start at 1 and increase on edits, undo, and redo. Selections use directed UTF-8 byte endpoints; the GUI adapter converts scalar indices. Full grapheme cursor behavior is a remaining view gate, not an achieved capability.
- libloading 0.8.9 loads real cdylibs at startup. Host/SDK meet through the implemented v1.0 C tables in plugin-api; the desktop does not link the plugin crate. Explicit plugin installations and manifests gate loading; Windows uses DLL-directory + System32 search flags, Unix uses RTLD_NOW | RTLD_LOCAL.
- The M0 command registry lives in plugin-host. A standalone commands crate and editor-view crate will be extracted when M1 needs shared routing/view responsibilities; no empty placeholders were created.
- Rust FFI boundaries convert unwindable panics to fixed status values. Unknown native state or failed shutdown retains the library, host table, and context until process exit. Disabling only removes dispatch; updates require restart.
- Native GUI verification, older released-SDK compatibility, performance budgets, safe file saving, and platform release support are not established by the current automated tests.

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

Do not turn every crate into a dynamic library. Frequent internal operations involving documents, layout, and input continue to use Rust types. Extension boundaries use a narrow C ABI. Built-in search supplies baseline functionality, while plugins provide replaceable extension commands so startup does not require third-party DLLs.

## Technology Choices and Validation Gates

| Area | Proposed choice | Rationale and required validation |
|---|---|---|
| GUI | egui + eframe | Efficient Rust desktop prototyping; M0 must validate Windows Chinese IME, scaling, clipboard behavior, and shortcuts |
| Text model | ropey | Candidate chunked text model; benchmark large-file editing, indexing, and snapshot costs |
| Editor view | Prototype interaction with TextEdit, then implement a core-driven virtualized view | A conventional string widget does not establish large-file capability; do not retain two independently editable models |
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
- [egui TextEdit](https://docs.rs/egui/latest/egui/widgets/text_edit/struct.TextEdit.html)
- [libloading](https://docs.rs/libloading/latest/libloading/struct.Library.html)

These references document tool capabilities and boundary requirements. The technology choices, directory structure, and milestones above are project design proposals.
