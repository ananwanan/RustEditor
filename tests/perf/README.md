# M0 Windows GUI performance protocol

This protocol separates measured process statistics from actual displayed GUI performance. A successful headless smoke test, a Core transaction timer, an egui update duration, a main-window handle, and a submitted frame cannot establish input-to-present latency or startup to an editable presented frame.

## Build and conditions

Use Windows 11 x64, at least four CPU cores, 16 GiB RAM, and an SSD for the roadmap reference comparison. Record the exact Windows build, CPU, RAM, GPU/driver, disk type, monitor resolution/refresh rate, scaling, power mode, remote/local session, background workload, and installed font choices. Keep power mode, display, viewport size, fonts, and plugin mode constant across samples. Do not report a remote or virtualized session as a physical-machine result.

Build outside the timed interval:

```powershell
cargo build --release --locked -p rusteditor-desktop
```

Use the executable directly with `--no-plugins --empty` for the empty-startup and idle-memory workloads. Resolve a non-default Cargo output directory with `cargo metadata --no-deps --format-version=1 --locked`; do not assume `target/release` if a target directory is configured. Record the revision, binary SHA-256, build flags, Rust toolchain, and dirty state. The release profile uses thin LTO. Close any existing measured editor windows and avoid other GPU-intensive work.

## Idle memory and window-discovery diagnostic

From the repository root, on an unlocked interactive desktop:

```powershell
powershell -NoProfile -File tests/perf/windows_gui.ps1 -Binary target/release/rusteditor-desktop.exe -Output target/m0-perf/windows-warm.json -DisplayScalePercent 100 -Samples 5
```

Use `-EnvironmentNote` to record local/remote session, monitor refresh rate, power mode, and known background load, without personal identifiers. The script records hardware metadata without hostname, username, serial numbers, or user profile paths. Its display scale and build mode fields are explicitly operator-reported.

The script performs one untimed GUI priming launch followed by five measured launches. It polls for a nonzero process main-window handle every 10 ms, waits five seconds, then samples `WorkingSet64` and `PrivateMemorySize64` ten times at 100 ms intervals. It closes only the untouched process it launched. A failure, timeout, or forced cleanup remains in the raw report and invalidates a complete run. Cleanup exceptions are retained in the sample, and `run_failure` records an interrupted measurement sequence. Do not interact with these windows while sampling. The report is exclusively reserved before launches; existing reports are never overwritten, including by concurrent runs.

The JSON retains every memory sample and launch-to-window observation, plus nearest-rank p50/p95 diagnostic statistics and idle memory summaries. Windows process working set is the resident-memory analogue used here; private bytes are reported separately. GPU allocations and total system memory are outside this counter. Compare the maximum of the idle working-set samples with the `< 150 MiB` budget. An environment below the reference specification is evidence for that environment only.

**Launch-to-window observation is not the roadmap startup metric.** It excludes any proof that the editor has painted or can process input, and polling/scheduling adds uncertainty. The script leaves actual editable startup, cold startup, and input-to-present latency Pending. Do not compare this diagnostic with the `< 1 second` editable-startup budget.

Generated JSON and recordings belong under ignored `target/` or an external evidence folder. Commit only intentionally reviewed, non-personal acceptance data.

## Actual warm and cold startup

Use a high-speed camera with known frame rate that captures the display and a synchronized launch indicator, or a Windows ETW/WPR trace that correlates process launch, the editor's first painted editable frame, and display presentation. Record tool versions, clock source, event correlation, refresh rate, and measurement uncertainty. A first `Present` can contain a splash/blank frame; inspect the corresponding displayed frame and demonstrate that a key input is accepted. A software update marker alone does not establish a displayed frame.

For warm startup, prime by launching and closing the same binary once. Record at least 20 independent direct launches and their raw launch-to-editable-presented-frame intervals. Report sample count, median, p95 using nearest rank, minimum, maximum, and capture uncertainty. Compare warm startup with `< 1 second`; retain missed frames and failures in the record.

For cold startup, use a separately recorded reboot before each sample, allow a fixed idle interval, and confirm the editor and relevant assets have not been launched since reboot. Record boot time, idle interval, startup applications, cache condition, and sample count. OS prefetch may still populate caches; state that limitation. A first launch during a warm session is not a cold-cache measurement. Do not clear global caches or change system settings silently. Report cold results separately; the roadmap does not specify a cold-start budget.

## Input-to-present latency

Prepare exactly 1,048,576 UTF-8 bytes in the scratch editor by native clipboard paste, without adding file I/O to M0. The stdlib-only generator repeats `hello 你好 é 👩🏽‍💻 🇨🇳 1️⃣` with the selected newline, then adds ASCII `a` bytes to reach the exact size without splitting a Unicode character:

```powershell
python tests/perf/workload.py --variant lf --output target/m0-perf/one-mib-lf.txt
python tests/perf/workload.py --variant crlf --output target/m0-perf/one-mib-crlf.txt
python tests/perf/workload.py --variant long-line --output target/m0-perf/one-mib-long-line.txt
python -B -m unittest discover -s tests/perf -p test_workload.py
```

Each command emits JSON with byte/scalar counts, SHA-256, newline counts, and the byte count/hash after CRLF-to-LF normalization. Output uses exclusive binary creation and never overwrites an existing fixture. `--bytes` permits explicit sizes up to M0's 4 MiB limit. Use the **LF variant** for an exact 1 MiB ordinary workload even after native clipboard newline normalization; the **CRLF variant** becomes smaller when normalized, and the **long-line variant** contains no newline. Keep those cases separate.

Record the UTF-8 byte count and SHA-256 before paste and copied back afterward. Load the generated fixture in a native text application and copy it into the scratch editor; M0 itself still has no file-opening feature. Clipboard integration may normalize CRLF to LF; retain the resulting byte count/hash and label the normalized workload rather than claiming the original bytes were preserved.

Measure at least 200 ordinary key insertions at a fixed caret location/viewport with a fixed cadence (for example, 5 Hz), after layout has settled. Correlate each physical/input-device event with the first displayed frame containing its change, using a high-speed camera with an input indicator or hardware timestamps synchronized with presentation capture. At 240 fps, one frame is about 4.17 ms; state quantization and any display scanout/input-indicator uncertainty. An external input indicator must represent the input event consistently, not a delayed UI marker. Software injection timestamps, Core transactions, screenshot polling, and CPU paint completion alone do not qualify.

Alternatively, collect a Windows ETW/WPR input and graphics trace with a supported presentation-analysis tool; document the exact trace profile, tool version, exported events, input-to-frame association, clock conversion, and whether the reported endpoint is submitted, displayed, or estimated. Only displayed-frame evidence supports this gate. If the chosen tool cannot associate changed text with a displayed frame, keep the gate Pending.

Installed WPR/WPA tools and available graphics profiles do not establish that correlation. Validate the chosen profile and analysis on a changed-text sample before collecting acceptance data. Run tracing separately from idle-memory sampling, record its overhead, and preserve any existing recording rather than stopping or replacing it.

Report all raw event/presentation pairs, missed/dropped/coalesced input, median, p95 nearest rank, worst case, and measurement uncertainty. Compare the ordinary 1 MiB UTF-8 workload with `< 16 ms p95`. Repeat and label Chinese IME, combining/ZWJ text, CRLF/LF, and an extremely long line separately; a literal-text injection does not validate native IME. Do not flatten all workloads into one percentile or replace a failed result with a Core timer.

M0 has no GBK/UTF-16 decoding, asynchronous file opening, or 10/100 MiB large-file mode. Those roadmap scenarios remain future milestone work, not fabricated M0 results. If the bounded full-document galley misses the 1 MiB target, record actual latency and the architectural consequence before M1.

### Shortest remaining manual capture

1. Record the final binary hash, revision, build, display configuration and original input/scale settings. Complete the [Windows manual gate](../../docs/acceptance/M0.md#windows-manual-gate) with physical input, explicitly selecting Microsoft Pinyin and recording its version and compatibility setting. Restore the original settings afterward.
2. Position a high-speed camera to capture the display and an input/launch indicator with a documented timing relationship. Verify frame rate and the visibility of both before measurement.
3. Prime the final `--no-plugins --empty` binary once, then capture at least 20 independent warm launches. Identify the first editable displayed frame and demonstrate accepted input; retain every raw interval and uncertainty.
4. Paste and copy back the generated exact 1 MiB LF fixture, verify its bytes/hash, then capture at least 200 ordinary key events and their first changed displayed frames at a fixed caret/viewport/cadence. Repeat native Microsoft Pinyin and long-line cases as separate runs.
5. Export raw input/display pairs, missed or coalesced events, nearest-rank percentiles, worst case and timing uncertainty. Keep any case without reliable correlation Pending. A verified ETW input/display chain may replace the camera only after it meets the same endpoint and association requirements.

The available Windows UI automation uses SendInput. Its observations are supplemental: disclose that method and do not infer physical-input acceptance from automated key presses, Unicode injection, screenshots, or headless egui events. Physical native acceptance and presentation capture remain separate from these diagnostics.

## Internal render CPU and allocation diagnostics

These headless diagnostics exercise the actual `EditorView::show` and existing Unicode generator. They do **not** create a native window, synthesize a native IME, tessellate a frame, execute GPU work, or observe display presentation. Never compare their timings with the input-to-present or startup budgets above.

```powershell
cargo build --release --locked -p rusteditor-editor-view --example render_diagnostics
python -B tests/perf/render_diagnostics.py --binary target/release/examples/render_diagnostics.exe --output target/m0-perf/render.json --label local-render --samples 30 --font C:/Windows/Fonts/msyh.ttc --emoji-font C:/Windows/Fonts/seguiemj.ttf --environment-note "Record power mode, background workload and session conditions here."
```

Resolve custom target directories through Cargo metadata as above. Font arguments are optional; omitting them uses bundled egui defaults and must be labeled separately. The example accepts only bounded UTF-8 fixtures; it introduces no editor file-opening feature. The runner reuses `workload.py` for empty, exact 1 MiB LF, normalized CRLF (1,028,016 bytes), and exact 1 MiB long-line inputs. It records hashes, raw samples, nearest-rank p50/p95, OS/hardware, toolchain, source fingerprints, build mode and binary hashes. Outputs use exclusive reservation; interrupted/failed runs remain explicit. Keep the same release harness, font hashes, viewport, logical scale, sample count and environment for before/after runs; build outside all timed intervals and avoid concurrent builds.

`--source-root` defaults to the repository root. When measuring a separately built baseline archive, pass its actual source directory so the recorded normalized source hashes describe that build. The runner records the invoking checkout's HEAD and dirty state separately. A baseline archive must contain the same final example and diagnostic-only dev dependency/lockfile as the changed build; only production view sources differ. The binary reports `debug_assertions` and debug binaries are rejected. Source fingerprints, binary hashes and the recorded build commands together establish provenance; a dirty HEAD label alone does not.

The example records 19 scenarios per workload: 11 view-frame scenarios and eight isolated probes. Each show sample includes `Context::run`, view event processing and output destruction. Idle, caret, changing selection, stable whole-document selection, editing, editing after idle, preedit changes/repeats, commit/cancel and Windows clear-before-commit ordering are separate scenarios. Setup/warmup and preedit setup for commit/cancel are outside the measured sample. Every commit/cancel sample explicitly requests focus before preedit setup, asserts that a composition exists, and asserts that the measured event consumes it. Alternating edit/Backspace returns to the original bytes every other sample; this is useful for cache reuse diagnosis but does not model arbitrary continuous typing. The edit-after-idle case allows egui's unused entries to expire.

The original four isolated probes time text copies, owned layout-cache hits, and selection mutation, and record actual Galley sharing and cache eviction. Layout-cache hit timing includes its owned text copy and hash, rather than pretending it is cold shaping time. Selection-mutation timing includes painting and destruction. Four appended probes preserve the preceding scenario order: `selection-drop-first` and `selection-drop-all` paint outside the timed interval and measure only destruction of the resulting selected Galley; `payload-accounting-base` and `payload-accounting-base-selected` measure the exact allocation-free production accounting helper on the base alone or the base plus its painted selection. Accounting setup, painting and selected-Galley destruction are outside those accounting intervals. The accounting probes report `accounted_retained_bytes`; they omit the cache's separate font-epoch Galley and are not total-process memory measurements.

Historical 2026-10-28 reports retain their original 15 scenarios and raw values. Their `ime-cancel` series did not restore focus after Escape: after the first cancellation, subsequent samples largely measured an unfocused no-op. Preserve those results with this limitation; do not treat their percentiles as repeated-cancellation costs or compare them directly with the corrected series. New before/after comparisons must use the same corrected harness on both production variants.

The standalone example forwards `GlobalAlloc` to Rust's System allocator and counts successful requested bytes/calls. `allocated_bytes` and `allocation_calls` count new payload requests; `deallocated_bytes` and `deallocation_calls` count released payloads during the measured interval. A successful reallocation counts the entire old payload as released and the entire new request as allocated, including an in-place resize; a failed reallocation contributes neither. These are requested payload counters, not a count of OS memory operations. Live/peak counters track Rust allocation payloads, including fonts, egui state and retained JSON recording data; they exclude allocator overhead, native/GPU allocations and RSS/working set. `peak_extra_live_bytes` is the additional live payload over the pre-sample value. Absolute live totals accumulate the recorder across scenarios, so compare the same scenario/sample order. The view's cache cap is a separate retained-payload accounting rule, not a total-process memory bound. Keep raw data and disclose retained memory tradeoffs alongside reduced allocation churn. Historical reports without deallocation fields do not establish zero deallocation.

CPU samples include atomic allocation bookkeeping. Identically instrumented before/after runs diagnose changes; they are not uninstrumented production-frame budgets. The [M0 render validation record](../../docs/acceptance/M0.md#m0-render-diagnostics-and-limited-optimization) retains both the selected optimization and a rejected cache variant, including its edit regressions.
