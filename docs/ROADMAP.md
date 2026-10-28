# Development Roadmap and Acceptance Criteria

M0 implementation is available; **acceptance remains incomplete**. The workspace, bounded editing core, native desktop prototype, real plugin loading, SDK, text-tools, xtask, and automated failure tests exist. See the [acceptance record](acceptance/M0.md). Native Windows checks at 125% scale provide partial Sogou IME, clipboard, plugin, no-plugin, and close-confirmation evidence. Microsoft Pinyin, the required native 100%/150%/200% DPI cases, and displayed-frame performance gates remain Pending. Older SDK compatibility is not required. M1 remains blocked until mandatory M0 acceptance passes; M1–M6 have not started. Dependencies are locked for reproducible builds, while the GUI and ABI remain provisional. Estimates below are rough figures for one full-time developer, not delivery commitments.

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

Use the [M0 Windows performance protocol](../tests/perf/README.md) for reproducible empty-buffer memory sampling, warm/cold startup capture, and a precisely sized 1 MiB Unicode workload. The harness runs the built GUI with `--no-plugins --empty` and retains raw process memory and launch-to-window observations. Window discovery is not editable-state startup or frame presentation and must not be compared with the startup budget. Actual warm/cold editable-frame startup and input-to-present latency remain Pending until correlated display evidence is recorded. The [acceptance record](acceptance/M0.md) owns actual measurements and hardware qualifications. M0 still has no file opening, GBK/UTF-16 decoding, or 10/100 MiB modes; those scenarios remain later milestone work.

The M0 render follow-up measures internal CPU/allocation costs on the same Unicode workloads and limits optimization to repeated event layouts and stable selection painting. It preserves egui's initial lookup each frame after a broader cache bypass measurably regressed editing. These diagnostic results do not satisfy displayed-frame performance acceptance or complete M0; M1 remains blocked.

## Validation Strategy

Core: edit/undo/redo invariants, randomized edits compared with a simple reference model, and Unicode boundaries. File I/O: encoding round trips, unencodable characters, external changes, and preservation of originals after write failures. Search: cross-rope-chunk matches, zero-length matches, replace-all, and cancellation. Syntax: revision races. Plugins: real dynamic library loading and subprocess failure tests. UI: manual Windows IME, DPI, shortcut conflict, tab closing, clipboard, and recovery records, followed by platform-specific expansion.

Each milestone must include implementation, necessary tests, build commands, actual acceptance evidence, and an updated milestone status. Empty crates or screenshots alone do not establish completion.

## Remaining M0 Tasks

1. Complete the Windows manual checklist with Microsoft Pinyin composition/candidate positioning and native 100%/150%/200% DPI, including scrolling, window movement, and monitor transitions. An attempted Settings launch exposed no controllable window in this session; these cases remain Pending. Preserve the narrower 125% Sogou, clipboard, plugin/no-plugin, and close-confirmation observations in the acceptance record.
2. Complete Windows grapheme/font validation for combining marks, emoji/flag/ZWJ sequences, CRLF, word/vertical navigation, and pointer hits. Automated boundary, input-order, clipboard, and synthetic IME regressions exist. Validation fixes cover the scrolled candidate rectangle, cancellation/session tracking against late commits, and Windows clear-before-commit ordering; they do not replace the remaining native checks.
3. Target the current experimental SDK/ABI. Older SDK compatibility and frozen historical SDK fixtures are explicitly out of scope; keep version/size/capability validation and real-library failure tests.
4. Retain eframe/egui 0.33.3 provisionally and finish the GUI decision using the remaining native and displayed-frame evidence. Apply the performance protocol on the specified Windows hardware; publish actual startup/input results and any missed budgets before marking M0 complete or starting M1. Automated Ubuntu/Windows DLL/core checks already pass; the acceptance record links their exact scope. Preserve current dependency, transaction, revision, and ABI safety boundaries without adding historical compatibility or hot unloading.

## Decisions Still Open

Freeze the GUI after M0. Hot unloading is outside the initial release. Publish exact supported OS versions and package policies after testing. Choose the project license before formal release. Plugin v1 supports text commands only; future UI extensions should use host-provided declarative panel APIs rather than passing GUI objects across DLL boundaries.
