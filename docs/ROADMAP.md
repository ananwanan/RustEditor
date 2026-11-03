# Development Roadmap and Acceptance Criteria

M0 is **accepted with maintainer-approved evidence exceptions as of 2026-11-03**. M1 is in progress. See the [M1 acceptance record](acceptance/M1.md) for M1.1 results. M2–M6 have not started. The [sign-off and Windows revalidation](acceptance/M0_2026-11-03.md) records the maintainer's confirmation that the missing Pinyin/native-DPI/input-to-present tests were performed and should pass despite unretained records. It separately retains 42 local Rust tests, 4 Python tests, native 125% clipboard/shortcut/close observations, actual release memory samples and [green Windows/Ubuntu CI](https://github.com/ananwanan/RustEditor/actions/runs/41481683607). No missing numerical or per-scenario results are invented. eframe/egui 0.33.3 and the current experimental C ABI are retained for M1 with documented font, full-layout and memory limits. Older SDK compatibility is not required. Estimates below are rough figures for one full-time developer, not delivery commitments.

| Milestone | Estimate | Deliverables and completion criteria |
|---|---|---|
| M0 Technical validation | 1–2 weeks | Create the minimal workspace/core/api/host/sdk/desktop/text-tools/xtask path; dynamically load an example DLL and uppercase a selection; run without plugins; manually validate Windows Chinese IME composition and candidate positioning, 100%/150%/200% DPI, clipboard behavior, and shortcuts; record GUI and ABI decisions |
| M1 Basic editing | 2–4 weeks | Document model, core-driven view, tabs, line numbers, selection, undo/redo, open/save, modified state, and close confirmation; test Chinese/emoji/combining characters, empty files, CRLF/LF, read-only files, and save failures; remove duplicate mutable state between temporary string widgets and core |
| M2 Everyday text features | 2–3 weeks | Literal/regex find and replace, next/previous match, and replace-all as one transaction; UTF-8/BOM, UTF-16, and GBK; unencodable-character prompts; external modification conflicts; configuration, shortcuts, and basic session recovery; byte-range and Unicode boundary regressions |
| M3 Code editing | 3–5 weeks | Incremental highlighting, indentation, bracket matching, folding, automatic indentation, and split views; initial Rust/C/C++/JSON/TOML support; cancellable background parsing that does not block input or overwrite newer document results |
| M4 Usable plugin management | 2–3 weeks | Complete manifests, enable/disable UI, logging, and SDK documentation; line-sorting and case-conversion plugins; validate the current SDK/ABI, invalid plugins, duplicate registrations, and exit cleanup; plugin updates take effect after restart |
| M5 Performance and reliability | 2–4 weeks | Large-file mode, directory search, long-line degradation, recovery snapshots, corrupted recovery files, full disks, and file-lock tests; publish reproducible benchmarks and known limitations |
| M6 Initial release | 1–2 weeks | Windows portable package, version information, license inventory, and release checksums; validate complete workflows on a clean physical Windows machine; claim Linux/macOS support only after separate validation |

Dependencies: M0 → M1 → M2 → M3 → M4 → M5 → M6. Dynamic loading is established in M0; M4 delivers user-facing management rather than introducing loading for the first time.

## M1 Implementation Plan (Approved Sequence)

**Planning decision (2026-11-03):** Start M1 with bounded, single-file UTF-8 opening, then reliable saving and modified-state semantics, multi-document tabs, daily editing UI, and full acceptance. The [M1 acceptance record](acceptance/M1.md) tracks implementation and verification against this sequence. Preserve the existing M0 core-driven view, Unicode/grapheme behavior, undo/redo, and native plugin/ABI tests. The M0 `String` document cap remains 4 MiB; a Rope migration, large-file mode, recovery, and non-UTF-8 encoding conversion are not prerequisites for M1.

| Subtask | Scope | Exit gate |
|---|---|---|
| **M1.1 Bounded UTF-8 file opening** | One document, `--open PATH`, a dedicated file I/O crate, strict UTF-8 without BOM, exact LF/CRLF preservation, and visible filename | Real files load into the existing `Document`/`EditorView`; errors never display truncated or replacement-character text; M0 regressions pass |
| **M1.2 Safe save and modified state** | Save/Save As, explicit saved baseline, immutable save snapshot, same-directory temporary file and platform-validated replacement, errors and Save/Discard/Cancel | Original data survives failed saves; successful save clears modified state only for saved contents; edits made during a save remain dirty |
| **M1.3 Multiple documents and tabs** | Per-tab document identity, selection and history, active-document routing to plugin commands, independent close handling | Switching/closing tabs cannot mix text, revisions, plugin targets or undo stacks |
| **M1.4 Everyday editing UI** | Line numbers, filename/encoding/line-ending and modified indicators, status updates, Open/Save/Save As actions and shortcuts | GUI workflows work with IME, Unicode selections, and the existing core-driven view; no duplicate mutable text state |
| **M1.5 M1 acceptance** | Cross-platform automation plus Windows native manual evidence for open/edit/save/tab/close workflows | All M1 roadmap scenarios pass with recorded commands, revisions, evidence and documented limitations |

### M1.1 — First implementation slice

**Goal:** Add the smallest end-to-end disk-to-editor path without repeating M0 editing, plugin or GUI infrastructure.

1. Add `crates/file-io/Cargo.toml` and `crates/file-io/src/lib.rs`; register `rusteditor-file-io` in the root `Cargo.toml`, update `Cargo.lock`, and declare it in `apps/desktop/Cargo.toml`. Keep this crate UI- and plugin-independent. An initial `open_utf8(&Path) -> Result<String, OpenError>` is sufficient; define typed I/O, invalid-UTF-8, unsupported-BOM and size-limit failures.
2. Read at most `MAX_DOCUMENT_BYTES + 1` bytes before validating and constructing the document. Reject files larger than the current 4 MiB limit (do not truncate); decode strictly (do not use lossy replacement). Accept empty files and valid Chinese, emoji and combining sequences, preserve line endings exactly, and do not silently strip BOMs or convert encodings. Reading read-only files is supported; writing remains out of scope. Avoid unbounded allocations or unnecessary full-file copies.
3. Extend `apps/desktop/src/main.rs` options with `--open PATH`, incompatible with `--empty` and `--smoke-test`. On successful read, initialize `Document::new(DocumentId(1), text)` before creating the existing `SharedDocument` and `PluginHost`. Fail cleanly on missing paths, read errors, invalid UTF-8 or oversized files instead of silently opening an empty scratch buffer. Preserve default preview, `--empty`, `--no-plugins` and headless smoke behavior.
4. Update `apps/desktop/src/view.rs` to show the loaded filename and clearly state that disk saving is not yet available. Continue to use the authoritative core `Document` and existing `EditorView` for editing; do not add a second editable `String`, duplicate undo history, a file picker or tabs. Keep the existing protective close confirmation, but do **not** equate `revision > 1` with proper saved/dirty semantics: revision increases on undo/redo, so M1.2 must introduce explicit save state.
5. Add unit tests in the file I/O crate for empty, UTF-8 Unicode, LF/CRLF byte fidelity, read-only input, invalid UTF-8, unsupported BOM, nonexistent input and the exact 4 MiB boundary. Exercise the real Windows GUI with an existing Rust fixture; verify editor edits, undo/redo and the existing plugin pathway. Update `README.md`, this roadmap and a new `docs/acceptance/M1.md` with **actual** test results only when implementation exists; keep the milestone marked in progress until all M1 gates pass.

**Implementation paths:** `Cargo.toml`, `Cargo.lock`, `crates/file-io/{Cargo.toml,src/lib.rs}`, `apps/desktop/{Cargo.toml,src/main.rs,src/view.rs}`; documentation: `README.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, and `docs/acceptance/M1.md`. No plugin ABI change is required.

**Deliberate M1.1 exclusions:** Save/Save As, file dialogs and Ctrl+O; tabs and line numbers; UTF-8 BOM, UTF-16 and GBK conversion (M2); external-change conflict handling and recovery (M2); large-file mode (M5). Do not change `eframe/egui` or migrate the core to Rope in this slice.

### M1.2–M1.5 — Implementation gates

- **M1.2:** Design saved state separately from monotonically increasing `Revision`; undoing back to saved contents should be recognized appropriately. Save an immutable text snapshot, associate successful completion with that snapshot, and never mark intervening edits saved. Use a same-directory temporary file, write/flush/sync and a Windows-validated replacement strategy; retain original bytes and modified state on failures. Cover read-only permissions, locks, disk/full or simulated I/O failure, empty documents, LF/CRLF preservation, Save As and close cancellation. Do not silently overwrite external changes; detailed external-change conflict workflows remain M2.
- **M1.3:** Give each tab an independent `DocumentId`, `Document`, `EditorView`, file association and close state. The M0 plugin host currently holds one `SharedDocument`; refactor its active-document routing safely, without reloading plugins or weakening revision validation. Test tab switching while composing with IME, independent histories and commands against the correct active document.
- **M1.4:** Implement visible line numbers, file/modified indicators and basic menu/shortcut actions. Keep text in `editor-core`, not an independently mutable egui text widget. Validate Chinese IME positioning, clipboard, Unicode/grapheme operations and Windows DPI after layout changes.
- **M1.5:** Re-run automated file/core/plugin/no-plugin tests and native GUI checks. Archive commands, environment, exact revision, observed outcomes, and unresolved platform limits in `docs/acceptance/M1.md`; do not infer a green GUI outcome from headless CI. Only declare M1 accepted when the full M1 milestone row and its failure scenarios are demonstrated.

### Local development and validation commands

Use the existing `cargo xtask` alias from `.cargo/config.toml` and the repository root. The [acceptance record](acceptance/M1.md) lists the actual results for these commands.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p rusteditor-file-io --locked
cargo test --workspace --locked
python -B -m unittest discover -s tests/perf -p "test_*.py"
cargo xtask verify
cargo run --locked -p rusteditor-desktop -- --no-plugins --open ".\tests\fixtures\probe.rs"
git diff --check
```

For the Windows GUI acceptance, open the provided Rust fixture and exercise Unicode edits, undo/redo, close confirmation and plugin behavior separately with staged plugins. Generate invalid-UTF-8, empty, LF/CRLF, read-only and >4 MiB test files under ignored `target/` for rejection/boundary checks. Record the results; do not commit generated test artifacts. Run the existing main-push/PR CI on Windows and Ubuntu. Review the feature on `feat/m1-utf8-open`.

## Performance Budgets (to Calibrate in M0/M5)

Use a recorded Windows 11 x64 test machine with at least four CPU cores, 16GB RAM, and an SSD. Record the exact hardware, build configuration, cold/warm startup conditions, and input data. Budgets are targets, not measurements.

| Scenario | Initial budget |
|---|---|
| Empty startup without plugins | Warm startup to editable state < 1 second; idle RSS < 150 MiB |
| Ordinary 1 MiB UTF-8 file | Input-to-render latency p95 < 16 ms |
| 10 MiB / 100k-line file | Input-to-render latency p95 < 33 ms; cancellable asynchronous opening |
| 100 MiB log | Restricted large-file mode opens in < 5 seconds with total RSS < 600 MiB; explicitly document disabled or limited capabilities |
| Extremely long lines / regex search | UI remains responsive; tasks can be cancelled; no unbounded intermediate results or layout work |

Measure end-to-end latency from input events to frame presentation. Core operation timing alone does not represent the user experience. Record Chinese, emoji, CRLF, GBK, UTF-16, long-line, and repeated-replacement workloads separately. If a budget is missed, record the cause and fallback behavior rather than silently changing acceptance criteria.

Use the [M0 Windows performance protocol](../tests/perf/README.md) for reproducible empty-buffer memory sampling, warm/cold startup capture, and a precisely sized 1 MiB Unicode workload. The harness runs the built GUI with `--no-plugins --empty` and retains raw process memory and launch-to-window observations. Window discovery is not editable-state startup or frame presentation and must not be compared with the startup budget. The [November 3 acceptance record](acceptance/M0_2026-11-03.md) owns the actual memory measurements and hardware qualifications. Native/display acceptance includes a maintainer-approved evidence exception; archived correlated startup/latency samples remain unavailable. The numerical budgets above are unchanged and are not independently demonstrated by attestation. M1.1 adds bounded UTF-8 opening. GBK/UTF-16 decoding and 10/100 MiB modes remain later milestone work.

## Validation Strategy

Core: edit/undo/redo invariants, randomized edits compared with a simple reference model, and Unicode boundaries. File I/O: encoding round trips, unencodable characters, external changes, and preservation of originals after write failures. Search: cross-rope-chunk matches, zero-length matches, replace-all, and cancellation. Syntax: revision races. Plugins: real dynamic library loading and subprocess failure tests. UI: manual Windows IME, DPI, shortcut conflict, tab closing, clipboard, and recovery records, followed by platform-specific expansion.

Each milestone must include implementation, necessary tests, build commands, actual acceptance evidence, and an updated milestone status. Empty crates or screenshots alone do not establish completion.

## M0 Closure and Follow-up

**Accepted on 2026-11-03 with evidence exceptions:** see the [gate dispositions and GUI/ABI decision](acceptance/M0_2026-11-03.md). `.gitattributes` resolved Windows formatting; both `34f4747` and the documentation-only `c65c271` have green Windows/Ubuntu CI. The subsequent acceptance documentation still needs the normal main-push/PR workflow when submitted.

1. Preserve the maintainer-attested Pinyin/native-DPI/input-to-present passes separately from directly observed results. Recollect source/binary identity, the complete native scenario matrix and raw display correlations during subsequent GUI/performance validation; the original archive cannot be reconstructed.
2. Carry forward incomplete emoji shaping, the bounded full-document galley and the small measured idle-memory margin. Retain Unicode/IME/event-order regressions and test new editing behavior against those boundaries. These limits do not establish large-file or release readiness.
3. Retain eframe/egui 0.33.3 and the current experimental SDK/ABI for M1. Keep version/size/capability/ownership/lifecycle validation and real-library failure tests. Historical SDK compatibility, hot unloading and unrelated future features remain out of scope.

## Decisions Still Open

The M0 sign-off retains the GUI for M1 with documented limits; future measured regressions may require reassessment. Hot unloading is outside the initial release. Publish exact supported OS versions and package policies after testing. Choose the project license before formal release. Plugin v1 supports text commands only; future UI extensions should use host-provided declarative panel APIs rather than passing GUI objects across DLL boundaries.
