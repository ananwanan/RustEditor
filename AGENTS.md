# Repository Instructions

## Scope and Required Reading

These instructions apply to the entire RustEditor repository. Read any more specific `AGENTS.md` files before changing their directories.

Before implementation, read:

1. [README.md](README.md) for project scope, current status, and documentation policy.
2. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for workspace boundaries and proposed technology choices.
3. [docs/PLUGIN_ABI.md](docs/PLUGIN_ABI.md) for the dynamic library contract.
4. [docs/ROADMAP.md](docs/ROADMAP.md) for milestone dependencies and acceptance criteria.

Explicit user instructions take precedence over this guide. Keep implementation, requirements, and acceptance evidence consistent. Document architectural changes and their rationale rather than silently replacing the planned design.

The repository initially contains planning documents only. Inspect the current tree and milestone status before starting work; planned paths, example signatures, and proposed commands are not evidence that code exists.

## Language

- Write and maintain all project documentation in **English**, including this file, README files, design documents, plans, contributor guides, acceptance records, and Rust documentation comments.
- Use English identifiers, explanatory code comments, commit messages, and pull request descriptions.
- Respond to the user in their preferred language; the documentation policy does not require English conversation.
- Preserve multilingual input support. Chinese text, emoji, and combining characters are valid test fixtures and editor content. Do not remove them to satisfy the documentation language policy.

## Product and Milestone Boundaries

- Build a standalone Rust desktop text and code editor inspired by Notepad++, with Windows 10/11 x64 as the initial target and Linux/macOS following validation.
- Do not introduce a WebView-based application or claim Notepad++ plugin binary compatibility.
- Prioritize Chinese IME behavior, reliable saving, responsive editing, and startup speed.
- Start with M0: the minimal workspace, document/selection transaction, real dynamic library loading, a text-tools plugin, and Windows input/DPI validation. Dynamic loading must work in M0, not be deferred to M4.
- Follow the M0–M6 dependencies in the roadmap. Work on the requested milestone without implementing unrelated future features or creating every planned crate as an empty placeholder.
- Treat egui/eframe, ropey, tree-sitter, regex, encoding_rs, and libloading as proposed choices until the relevant validation gates pass. Record GUI, dependency, toolchain, and ABI decisions when they are validated.
- A milestone is complete only when its implementation and required acceptance checks are supported by actual results. Empty crates, screenshots alone, and successful compilation alone do not meet functional acceptance criteria.

## Cargo Workspace and Dependencies

Use a virtual Cargo workspace with `resolver = "2"`. Establish shared package metadata, edition, MSRV, dependencies, and lints during M0, after checking toolchain compatibility. Commit the application workspace's `Cargo.lock` and keep dependency changes scoped to the task.

Create members progressively using the planned paths and package names:

| Path | Package | Boundary |
|---|---|---|
| `apps/desktop` | `rusteditor-desktop` | Executable, application assembly, windows, menus, tabs, and shortcuts |
| `crates/editor-core` | `rusteditor-editor-core` | GUI-independent documents, transactions, selections, revisions, and undo/redo |
| `crates/editor-view` | `rusteditor-editor-view` | Layout, rendering, hit testing, scrolling, and IME integration |
| `crates/file-io` | `rusteditor-file-io` | Encodings, line endings, safe saving, monitoring, and recovery |
| `crates/commands` | `rusteditor-commands` | Command IDs, routing, shortcuts, and enablement |
| `crates/search` | `rusteditor-search` | Literal/regex searches and replacement transactions |
| `crates/syntax` | `rusteditor-syntax` | Parsing, highlighting, and folding information |
| `crates/settings` | `rusteditor-settings` | Configuration, themes, sessions, and migrations |
| `crates/plugin-api` | `rusteditor-plugin-api` | Minimal C ABI types and constants |
| `crates/plugin-sdk` | `rusteditor-plugin-sdk` | Rust plugin wrappers, validation, and panic conversion |
| `crates/plugin-host` | `rusteditor-plugin-host` | Discovery, loading, negotiation, registration, and lifecycle |
| `plugins/text-tools` | `rusteditor-text-tools` | Example text-command extension built as a `cdylib` |
| `xtask` | `xtask` | Development staging, packaging, and acceptance tooling |

Enforce the dependency directions in the architecture document:

- The desktop assembles the application; the core never depends on the GUI or plugin implementation.
- The view, file I/O, search, and syntax layers may depend on the core. Keep their responsibilities out of the core.
- The plugin host depends on plugin-api and the host command/core interfaces. The SDK depends on plugin-api; plugins depend on the SDK.
- Keep plugin-api independent of the GUI and editor-core. Do not expose their internal types through the ABI.
- Prevent dependency cycles and avoid a shared crate that becomes a miscellaneous container for unrelated functionality.
- Internal libraries normally link into the host. Use dynamic libraries for extension boundaries, not for every workspace member.

## Dynamic Library Contract

Follow `docs/PLUGIN_ABI.md`; it is the detailed protocol specification. The protocol remains a draft until M0 implementation and tests establish it.

- Build native plugins with `crate-type = ["cdylib"]` and load them through libloading. Do not substitute Rust's `dylib` ABI for a stable extension protocol.
- Keep the entry symbol `rusteditor_plugin_entry_v1`, use `extern "C"` functions and `#[repr(C)]` structures, and follow the selected Rust edition's export requirements.
- Use explicit ABI headers, fixed-width status values, pointer/length buffers, and opaque handles. Do not pass `String`, `Vec`, `Result`, trait objects, Rust references, or GUI objects across the boundary.
- Validate ABI major/minor versions, structure sizes, buffer capacities, required capabilities, lengths, and UTF-8 boundaries before accessing optional data or applying edits. Minor versions may append fields; incompatible changes require a new major version.
- Specify ownership, allocation/freeing responsibility, borrowing duration, thread rules, and error behavior for every ABI operation. Never free memory through another side's allocator or retain call-scoped borrowed pointers.
- Catch unwindable panics on each Rust entry/callback boundary and convert them to status codes. No panic may unwind across FFI; do not claim recovery from aborts, OOM, invalid pointers, or access violations.
- Keep unsafe code confined to necessary loading and FFI boundaries. Document each unsafe operation's safety requirements and establish those requirements at its call sites.
- Route plugin edits through host transactions with DocumentId, expected Revision, and validated UTF-8 byte ranges. Reject stale requests and conflicting command registrations.
- Serialize v1 dispatch on the UI thread, prohibit reentrant mutations, and keep plugin commands short. Do not allow background plugin threads to call UI APIs directly.

### Discovery and Lifetime

- Discover plugins through explicit plugin directories and manifests. Validate target platform/architecture, IDs, capabilities, and normalized paths; reject directory escapes and duplicate IDs.
- Do not scan the working directory or PATH for plugins. Use controlled dependency-library search paths, especially on Windows. Do not automatically load plugins from untrusted directories.
- v1 loads at startup and applies updates after restart. Disabling stops dispatch; retain the library handle until normal exit. Do not implement runtime DLL replacement or hot unloading as part of v1.
- Before final library release, stop new calls, unregister commands/subscriptions, finish in-flight work, shut down and destroy plugin instances, and release callbacks. Function pointers and instances must not outlive the library.
- If shutdown cannot establish safe cleanup, retain the library and require a restart. Stage Windows DLL updates for replacement before the next startup.
- Native plugins have host-process privileges. Capability declarations are not a sandbox; crash isolation for untrusted plugins requires a separate subprocess/WASM design.

## Editing, Threading, and File Safety

- Keep one authoritative editable document model. Temporary GUI prototypes must not become a second independently mutable model.
- Apply edits through transactions, preserve undo/redo behavior, and associate results and saved state with DocumentId and Revision.
- Distinguish UTF-8 byte offsets, Unicode scalar indices, graphemes, and screen columns. Validate character boundaries; cursor movement and layout must handle Chinese, emoji, and combining sequences correctly.
- The UI thread owns mutable editing state. Run disk I/O, parsing, searches, and snapshot work in cancellable background tasks with bounded queues. Discard stale results.
- Save an immutable snapshot and mark only its revision as saved. Preserve the modified flag if newer edits exist or saving fails.
- Follow the temporary-file, flush/sync, and platform-specific replacement strategy. Preserve permissions and original contents on failure; handle file locks, read-only files, disk-full errors, and external modification conflicts explicitly.
- Keep recovery files separate from originals. Offer Save/Discard/Cancel for modified tabs and restore/discard choices for recovery data.
- Handle UTF-8/BOM, UTF-16 LE/BE, GBK, and CRLF/LF according to the roadmap. Encoding detection is advisory; do not silently discard or replace unencodable content.
- Support cross-chunk search matches. Avoid unbounded document flattening, layout work, search results, and undo history. Treat large files and extremely long lines as explicit performance cases with documented limits.

## Validation and Commands

Inspect the actual manifests, toolchain, CI configuration, and xtask help before running commands. While the repository is documentation-only, check Markdown links, formatting, language, and consistency; do not report nonexistent Cargo checks as passing.

Once the workspace and lockfile exist, use these baseline checks for Rust changes from the repository root, together with relevant feature-specific tests:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Use the repository's supported target/feature combinations. Do not assume `--all-features` is valid. Use actual implemented xtask subcommands; do not invent build or packaging commands.

Validation must cover the behavior affected by the change:

| Area | Required evidence for relevant changes |
|---|---|
| Core | Transaction and undo/redo invariants, revision conflicts, Unicode boundaries; randomized/reference-model comparisons when editing logic changes |
| File I/O | Encoding round trips, unencodable content, external changes, save failures, and preservation of original files |
| Search/syntax | Cross-chunk and zero-length matches, replacement transactions, cancellation, and stale-result rejection |
| Plugins | Build and load a real cdylib from a host that does not link the plugin crate; verify no-plugin startup and ABI/lifecycle failure cases |
| Compatibility | Test an older-SDK plugin against a newer host; one shared workspace build does not prove ABI compatibility |
| Windows UI | Record IME composition/candidate placement, 100%/150%/200% DPI, clipboard behavior, shortcuts, tab closing, and recovery as applicable |
| Performance | Reproducible workload, hardware, build settings, latency/memory measurements, and documented degradation against roadmap budgets |

Run deliberately crashing plugins in subprocesses. Unit tests alone do not establish DLL loading, Windows behavior, or cross-platform support. Record checks that cannot run and the reason; do not substitute estimates for measured results or claim manual verification without performing it.

Establish main-push/PR fmt, clippy, and test CI first, then expand platform builds according to the roadmap. Do not introduce automatic releases as part of unrelated work.

## Change and Delivery Discipline

- Inspect the branch, worktree, and existing changes first. Preserve unrelated user work; do not force-push or rewrite published history to deliver an ordinary change.
- Keep patches focused. Update affected English documents when behavior, architecture, ABI, commands, or milestone status changes. Include actual commands and acceptance results when completing a milestone.
- Do not mark unverified work complete, invent test output, or present roadmap budgets as achieved measurements.
- Keep build output, generated binaries, local plugin installations, credentials, and personal editor data out of source control. Commit intentional, reproducible fixtures only when needed.
- Package plugins by platform, architecture, and ABI. Review dependency, grammar, font, and icon licenses before distribution; do not assume a project license that has not been selected.
- Before committing, inspect the diff and run `git diff --check` plus checks appropriate to the change. Documentation-only changes do not require editor builds.
- When a commit/push is requested, complete it within the authorized scope and report the branch, commit, meaningful changes, checks performed, and any remaining validation limits.
