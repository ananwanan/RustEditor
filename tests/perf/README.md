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

Report all raw event/presentation pairs, missed/dropped/coalesced input, median, p95 nearest rank, worst case, and measurement uncertainty. Compare the ordinary 1 MiB UTF-8 workload with `< 16 ms p95`. Repeat and label Chinese IME, combining/ZWJ text, CRLF/LF, and an extremely long line separately; a literal-text injection does not validate native IME. Do not flatten all workloads into one percentile or replace a failed result with a Core timer.

M0 has no GBK/UTF-16 decoding, asynchronous file opening, or 10/100 MiB large-file mode. Those roadmap scenarios remain future milestone work, not fabricated M0 results. If the bounded full-document galley misses the 1 MiB target, record actual latency and the architectural consequence before M1.
