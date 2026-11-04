# Architecture Plan

Date: 2026-10-28; acceptance decision updated 2026-11-03. The M0 prototype implements the desktop/core/view/api/sdk/host/text-tools/xtask path and is **accepted with maintainer-approved evidence exceptions**. The [sign-off](acceptance/M0_2026-11-03.md) retains eframe/egui 0.33.3 and the current experimental C ABI for M1, distinguishing archived results from maintainer-attested unarchived tests. M1 is in progress. The [M1 acceptance record](acceptance/M1.md) tracks file opening, safe saving, independent tabs, the M1.4 editing UI and M1.5 native input fixes and outstanding gates. Locked dependencies and documented prototype limits remain in force.

## M0 Implementation Decisions

- Virtual resolver-2 workspace, Stable Rust, edition 2024, shared metadata/lints, and committed Cargo.lock. No project license has been selected; no distributable release is claimed.
- Retain eframe/egui 0.33.3 with the native Glow backend for M1 under the November 3 sign-off. Archived native Windows 125% observations cover scoped Sogou composition, clipboard, plugin/no-plugin behavior and close confirmation; the revalidation adds multiline Notepad round trips and Ctrl+Shift+Z. Microsoft Pinyin, native 100%/150%/200% DPI and input-to-present acceptance rely on the maintainer's report of completed tests without saved records. No missing per-scenario or presentation measurements are inferred. Incomplete emoji shaping and full-document layout remain known limitations. WebView/WASM backends are not used.
- A bounded UTF-8 String is sufficient for the M0 transaction/ABI experiment. Ropey, virtualized layout, and large-file capability remain unvalidated. Maximum document size is 4 MiB, with 256 undo entries / 8 MiB of retained text deltas.
- Document is authoritative. The independent editor-view renders an egui galley and routes ordered keyboard, pointer, clipboard, and IME events into core transactions. It retains no editable String or duplicate undo stack. A bounded transient IME overlay retains the source DocumentId/Revision/range without mutating the document; commit is one validated transaction, cancellation preserves the original selection, and stale/late commits are ignored. Failed/oversized paste preserves the original text, selection and history.
- M0 validation supplies the caret line as the native IME exclusion rectangle instead of the full document rectangle, preventing the document's height from displacing candidates. Regression tests cover its geometry after scrolling, cancellation/session tracking that rejects late commits, and the Windows empty-preedit-before-commit event order. A native Sogou check at 125% scale observed candidates remaining near the caret through a titlebar drag. Full Pinyin/native-DPI acceptance is by maintainer attestation; its detailed matrix is not archived.
- Core revisions start at 1 and increase on edits, undo, and redo. ABI selections use directed UTF-8 byte endpoints. The view converts egui scalar positions and snaps user selections/carets to Unicode extended grapheme boundaries using unicode-segmentation 1.13.3 (Unicode UAX #29). Plain left/right, Shift selection and Backspace/Delete treat combining sequences, ZWJ emoji, flags, keycaps and CRLF as units. Word/vertical and pointer targets are normalized to cluster boundaries. This is navigation behavior, not a claim of complex-script/emoji ligature shaping.
- libloading 0.8.9 loads real cdylibs at startup. Host/SDK meet through the implemented v1.0 C tables in plugin-api; the desktop does not link the plugin crate. Explicit plugin installations and manifests gate loading; Windows uses DLL-directory + System32 search flags, Unix uses RTLD_NOW | RTLD_LOCAL.
- The M0 command registry lives in plugin-host. The editor-view crate was extracted to close the scalar-navigation gap before M1; the commands crate remains planned. No empty placeholders were created. Font installation remains desktop assembly code: bounded outline parsing through ab_glyph 0.2.32, CJK and emoji fallbacks, explicit overrides, and diagnostics; additional fonts are not redistributed.
- Rust FFI boundaries convert unwindable panics to fixed status values. Unknown native state or failed shutdown retains the library, host table, and context until process exit. Disabling only removes dispatch; updates require restart.
- The experimental SDK/ABI targets the current version; older SDK compatibility and historical fixtures are not required. ABI safety checks remain in force. Automated tests establish the scoped core/DLL behavior, not native GUI, presented-frame budgets, safe file saving or release support. Maintainer-attested GUI/performance acceptance and directly measured evidence are recorded separately in the sign-off. Ubuntu/Windows DLL/core CI and a Linux X11 window smoke pass; font limitations remain documented.
- The [Windows performance protocol and memory harness](../tests/perf/README.md) record reproducible workloads and separate resident process memory from presentation metrics. `--no-plugins --empty` starts an actual empty scratch buffer for measurement. November 3 release sampling peaked at 149.7070 MiB working set across 50 samples. A discovered window handle is only a diagnostic; archived correlated editable-frame startup and latency samples remain unavailable despite acceptance sign-off. No Core timer or CPU frame duration substitutes for display evidence.

## M1.1 File Opening

The desktop reads the startup path before it creates the native window or loads plugins. The file I/O crate uses a read-only file handle and reads at most the core document cap plus one byte, even when the file grows after its size check. It validates the bytes before transferring the resulting String into the core Document. The desktop shares that Document with the existing view and plugin host. The desktop retains the startup path and captures the disk baseline from the same open handle that supplied the text.

The synchronous startup read is bounded in bytes and runs before an interactive UI exists. M1.1 used session-level discard confirmation. M1.2 ties close confirmation to the saved contents.

## M1.2 Saving and Modified State

- The core owns editable text and a bounded immutable saved snapshot. Each text edit, undo, redo, or acknowledged save refreshes a cached exact comparison with the saved bytes. Rendering reads the cached flag. Selection changes do not alter it. Revision remains monotonic and plugin edits use the same transaction path.
- The desktop holds the path and disk baseline separately from core state. One standard-thread job per document returns one result through its join handle. Inspection and writing run off the UI thread. Save As confirms the observed destination before replacement and updates the association after commit. The UI defers closing while a write is running and rechecks current contents before closing.
- The disk baseline includes file identity, length, timestamps, attributes, and exact bytes within the 4 MiB bound. The worker checks it before staging and again before commit. Destination symlinks and Windows reparse points, including in parent directories, are refused. Use a direct path without parent traversal. Save As can replace existing files up to 4 MiB after confirmation. New destinations use a no-overwrite filesystem operation.
- The worker exclusively creates a sibling temporary file, writes the snapshot bytes without newline conversion, and flushes and synchronizes the file before publication. File I/O errors before publication leave the previous file in place. Cleanup is best effort when a directory becomes unwritable or the process terminates.
- Windows uses local drive paths. It publishes a new file with `MoveFileExW` without copy or overwrite flags. Existing files use `ReplaceFileW` with a same-volume backup and flags that require security and attribute merges. Staging must match the original owner, group, DACL, and mandatory label. An inherited no-index attribute must match on both files. Unsupported attributes and hard-linked targets are refused. Audit SACL preservation is unverified. If a partial failure moves the original, restoration uses a no-overwrite move. If restoration fails, the error identifies the retained original. Windows has no directory fsync in this flow. [ReplaceFileW contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew) and [MoveFileExW contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).
- Linux uses rename for an existing file and publishes a new file with a hard link before unlinking the staging name. It preserves Unix mode bits and refuses targets with ownership mismatches, special mode bits, hard links, or extended attributes such as POSIX ACLs. An exclusive nonblocking `flock` coordinates with cooperating writers. New files use mode 0600 or stricter, and the parent directory is synchronized after publication. Other Unix targets refuse existing-file replacement until their metadata policy is implemented.
- Existing-file publication has a residual race against noncooperating writers and concurrent directory changes between the final check and commit. A post-commit directory sync or verification problem reports a warning. Unverified destinations require Save As and keep the prior saved-text baseline. Power-loss behavior remains unverified. Windows and Linux use different publication and synchronization primitives. Full external-change handling and recovery remain M2 work.

## M1.3 Tabs and Active Document Routing

- The desktop `Workspace` owns up to 32 `Tab` instances. Each tab owns one shared core `Document`, one `EditorView`, its path/disk baseline, one `SaveSession`, status, and close confirmation. IDs increase for the lifetime of the window and are never reused. Editor focus and egui scroll state use DocumentId-based IDs. No second editable text buffer is introduced.
- Repeated `--open PATH` arguments load independent tabs before the interactive window and before native plugins load. New creates an empty tab. Startup errors fail the complete launch rather than silently substituting a blank tab. M1.4 adds the separate in-window opening flow below. Each document retains the existing 4 MiB text and bounded history limits; the tab cap also bounds worker/result counts, not a measured process-memory budget.
- Each save session owns at most one inspection/write worker. Every completion, including failures and destination inspection, carries DocumentId. The UI polls every tab and resolves the owning ID before updating the association, disk baseline, or saved snapshot. Written snapshots are checked again for matching identity. Successful writes only acknowledge their captured contents; newer edits remain modified. Closed/unknown IDs are ignored and cannot name later tabs.
- Frame input is processed before save completions can remove tabs, preventing pending text from falling into a successor document. Closing a saving tab records a deferred close and keeps the tab and worker alive. Success rechecks current dirty/composition state; newer edits prompt again. Failure, a publication warning, or an unverified destination cancels automatic closing. No write is detached or interrupted during publication. Session destruction joins remaining work as an exit safeguard; forced process termination and stalled filesystems retain the M1.2 limits.
- Close prompts and Save As dialogs carry their owning document ID and title. They can refer to an inactive tab without changing the plugin target. Window close checks tabs in order; Cancel stops the remaining sequence, while already closed tabs stay closed. Closing the last tab leaves an empty workspace with New available. There is then no plugin document target.
- Plugin-host and all stable callback contexts share an optional active-document slot. Switching changes that slot only between serialized dispatches; native libraries, instances, registrations, and API tables remain loaded. Callbacks still validate document ID, revision, UTF-8 boundaries, capabilities, thread, and phase. A stale request for another tab is rejected even if both revisions match. Closing releases the host's reference to the departed document. The wire ABI/SDK version is unchanged.
- A pointer tab switch finishes the departing view's frame, resets IME state on both views, requests native IME cancellation, and focuses the destination. Delayed commits are ignored until a fresh Enabled or nonempty Preedit event. Keyboard application shortcuts remain suppressed during composition/commit frames. Synthetic composition transitions pass; native candidate/composition and the full DPI matrix remain acceptance work, as recorded in [M1](acceptance/M1.md#m13-multiple-documents-and-tabs).

## M1.4 Everyday Editing UI

- `editor-view` paints a gutter from the same unwrapped galley rows used for text, selection and hit testing. LF splits rows; CRLF counts once, an empty document has one row, and a trailing newline adds an empty row. The font's digit width reserves at least two digits and grows with the row count. Only visible line-number labels are painted. Full-document text layout and the 4 MiB cap remain unchanged; this is not virtualized large-file layout.
- Text origin, selection clipping, caret scrolling, pointer conversion and native IME exclusion rectangles include the gutter offset, in logical egui points. The gutter is anchored to the scroll viewport horizontally; text scrolls underneath a separate clip. Vertical positions come directly from the galley, including fallback-font row heights. Gutter clicks target the row start and Shift-click extends the selection. Synthetic 100%/150%/200% geometry checks and native 125% observations are distinguished in [M1.4 acceptance](acceptance/M1.md#m14-everyday-editing-ui).
- Read-only status metrics are cached by DocumentId, Revision and directed Selection. They retain no text or history. The active path, modified flag, encoding, observed newline styles, line/grapheme column and selection grapheme/byte counts follow the active core document. Columns are one-based grapheme positions, not screen columns; a tab counts once. Bare CR is reported explicitly but does not split the LF-based layout. No encoding or newline conversion is introduced.
- `Workspace` owns one `OpenSession` in addition to existing per-tab save sessions. File/Open, the toolbar, and Ctrl+O use a path-entry modal consistent with Save As, without a new file-dialog dependency. One standard thread uses the existing strict UTF-8 reader (4 MiB plus one overflow byte) and captures the disk baseline from the same handle. There is no pending request queue. The in-flight result reserves one of the 32 tab slots, and another Open is unavailable until it drains. Failed reads leave all document, selection, history and save state intact and show a path-specific error.
- The reader returns only owned text, path and disk baseline. The UI creates a fresh core Document and non-reused DocumentId after success and activates it through the existing plugin-host slot. Frame input and per-document save completions run first. Ready read results wait while IME input, a modal or window closing prevents activation; they cannot redirect queued text or a save result into a newly opened document. Closing ordinary tabs does not cancel an independent Open; closing the window does. Cancel Open discards the result and retains the worker slot until it finishes. Blocking OS reads cannot be interrupted. At process exit this read-only worker may detach; it owns no UI, document or plugin reference and cannot publish an edit. Existing save workers still join rather than detach.
- One availability calculation drives File/Edit menus and toolbar actions: New/Open need capacity, Save needs changed contents or a scratch destination, Save As needs an idle active tab, and Undo/Redo need active history. Close retains the existing deferred-save behavior. Application shortcuts and mutation buttons are suppressed during IME frames and modals. Focused-editor Undo/Redo remains in its ordered event stream; menu/toolbar actions and shortcuts outside editor focus use the same core history. Path-entry Enter does not submit during observed IME sessions or their event frames.

No dependency, core transaction, plugin routing or v1.0 wire ABI changes are required. Encoding conversion, search, recovery and large-file modes remain in their roadmap milestones.

## M1.5 Native Input Validation

New/Open/tab activation cancels the departing native IME context and immediately restores it for the arriving editor. In egui-winit 0.33.3, platform output updates IME permission only when its cached `allow_ime` changes. Explicit viewport commands bypass that cache: sending only `IMEAllowed(false)` during a focused-to-focused transition leaves Windows composition disabled while the adapter still caches true. The paired disable/enable commands retain cancellation without leaving that mismatch. Both views still reset their transient sessions and reject delayed commits until fresh IME input; no text, history, revision or plugin ABI moves into the GUI adapter.

A frame regression models the adapter's cached platform output followed by explicit viewport commands across New, tab switching and a real background Open. Removing the re-enable command makes it fail. Native Microsoft Pinyin revalidation confirms immediate composition after each transition. Windows 11 100%/150%/200%, clipboard and workflow observations are scoped in [M1.5 acceptance](acceptance/M1.md#m15-full-acceptance-attempt). Wheel scrolling during preedit can leave the native candidate popup at its prior position until the next composition key; the platform-output geometry tests alone do not establish that the OS popup followed it. This remains an acceptance blocker. A dynamically enlarged normal window can exceed the work area at 200%; maximization restores its complete layout. No toolkit upgrade, Win32 candidate-window override or speculative startup repaint was introduced.

## Product Scope

Support everyday text, log, and code editing: multiple tabs, line numbers, a status bar, find and replace, syntax highlighting, encoding conversion, session recovery, and plugins. Priorities are reliable Chinese input, safe file saving, startup speed, and responsive editing.

The first usable release includes explicit open/save policies for UTF-8, UTF-8 BOM, UTF-16 LE/BE, and GBK; CRLF/LF line endings; undo/redo; single-file search and replace; basic highlighting; and one dynamic library plugin. Encoding detection provides candidates and cannot guarantee correctness. A full IDE, debugger, plugin marketplace, binary file editing, and Notepad++ DLL compatibility are outside the first release.

## Workspace Responsibilities

The workspace grows progressively by milestone. The Cargo workspace defines build and code boundaries. Only extensions marked `cdylib` are loaded at runtime; other crates are compiled and linked into the host.

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

The M0 dependency audit with `cargo metadata --locked --no-deps --format-version 1` confirms eight implemented members: core and plugin-api have no dependencies; view depends on core and egui; SDK depends on plugin-api; host depends on core, plugin-api, and loading/manifest libraries; text-tools depends on SDK. Desktop assembles view/core/host and does not link text-tools. The command registry remains inside plugin-host until its planned extraction. M1.1 adds file-io as a ninth member. M1.2 adds target-specific Win32 and Linux bindings to file-io. Desktop uses it for startup reads and background saves.

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

The M0 sign-off retains the GUI for M1 with documented evidence exceptions and limits. Reassess Slint, native controls, or other toolkits if subsequent measured IME, accessibility, layout or performance results require it. Do not label an untested dependency version as the latest supported version. Commit Cargo.lock during implementation and record the MSRV, licenses, and target platforms.

## Core Data and Threading

- Identify documents and versions with DocumentId and monotonically increasing Revision values. Keep text, selections, encoding, and line-ending policies separate. Track modified state against the last saved revision.
- Apply edits as transactions and store undo/redo deltas. Distinguish bytes, Unicode scalar values, graphemes, and screen columns. The ABI uses UTF-8 byte offsets at character boundaries; UI cursor movement follows graphemes, and layout determines display width.
- The UI thread exclusively owns mutable editing state. Background tasks handle disk reads, parsing, searches, saves, and recovery snapshots. Tasks must support cancellation, use bounded queues, and validate document revisions before applying results.
- Saving captures an immutable snapshot. Completion acknowledges the written snapshot. Current contents remain modified if they differ from that snapshot.
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
