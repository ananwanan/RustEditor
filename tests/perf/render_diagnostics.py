"""Run bounded, headless internal rendering diagnostics, never presentation latency."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import tempfile
from datetime import datetime, timezone

from workload import describe, make_workload


ROOT = Path(__file__).resolve().parents[2]


def command(args, cwd=ROOT):
    return subprocess.check_output(args, cwd=cwd, text=True, encoding="utf-8").strip()


def fixtures():
    """Reuse the GUI fixtures; CRLF is measured after native clipboard normalization."""
    result = [("empty", b"", {"variant": "empty", "utf8_bytes": 0})]
    for variant in ("lf", "crlf", "long-line"):
        source = make_workload(variant, 1024 * 1024)
        data = source.replace(b"\r\n", b"\n") if variant == "crlf" else source
        metadata = describe(source, variant)
        metadata["measured_utf8_bytes"] = len(data)
        metadata["measured_sha256"] = hashlib.sha256(data).hexdigest()
        result.append(("crlf-normalized" if variant == "crlf" else variant, data, metadata))
    return result


def summarize(samples):
    """Nearest-rank p50/p95; preserve every original sample in the report."""
    result = {}
    for metric in ("cpu_ns", "allocated_bytes", "allocation_calls", "live_bytes_before",
                   "live_bytes_after", "peak_live_bytes", "peak_extra_live_bytes"):
        values = sorted(sample[metric] for sample in samples)
        result[metric] = {"p50": values[math.ceil(len(values) * .5) - 1],
                          "p95": values[math.ceil(len(values) * .95) - 1],
                          "min": values[0], "max": values[-1]}
    return result


def environment():
    result = {"os": platform.platform(), "machine": platform.machine(),
              "logical_processors": os.cpu_count(), "python": platform.python_version()}
    if os.name == "nt":
        script = ("$cpu=Get-CimInstance Win32_Processor; "
                  "$os=Get-CimInstance Win32_OperatingSystem; "
                  "$gpu=Get-CimInstance Win32_VideoController; "
                  "[ordered]@{cpu=@($cpu|ForEach-Object {$_.Name}); "
                  "cores=@($cpu|ForEach-Object {$_.NumberOfCores}); "
                  "ram_bytes=[uint64]$os.TotalVisibleMemorySize*1024; "
                  "os_caption=$os.Caption; os_version=$os.Version; "
                  "gpu=@($gpu|ForEach-Object {[ordered]@{name=$_.Name;driver=$_.DriverVersion}})}|ConvertTo-Json -Depth 4")
        result["windows"] = json.loads(command(["powershell", "-NoProfile", "-Command", script]))
    return result


def source_hashes(source_root):
    """Normalize checkout newlines; stamp the actual build source directory."""
    files = [source_root / "Cargo.lock", source_root / "crates/editor-view/Cargo.toml"]
    files.extend(sorted((source_root / "crates/editor-view/src").glob("*.rs")))
    files.append(source_root / "crates/editor-view/examples/render_diagnostics.rs")
    return {path.relative_to(source_root).as_posix():
            hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()
            for path in files}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--label", required=True)
    parser.add_argument("--samples", default=20, type=int)
    parser.add_argument("--font", type=Path)
    parser.add_argument("--emoji-font", type=Path)
    parser.add_argument("--environment-note", required=True)
    parser.add_argument("--source-root", type=Path, default=ROOT,
                        help="Source directory used to build the binary, for normalized hashes")
    args = parser.parse_args()
    if not 1 <= args.samples <= 1000:
        parser.error("samples must be between 1 and 1000")
    binary = args.binary.resolve(strict=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Reserve before running anything. An interrupted report remains explicit.
    with args.output.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write('{"status":"incomplete"}\n')
        stream.flush()
        report = {"status": "running", "label": args.label,
                  "kind": "internal-cpu-allocation-diagnostic", "presentation_latency": False,
                  "started_utc": datetime.now(timezone.utc).isoformat(),
                  "git_commit": command(["git", "rev-parse", "HEAD"]),
                  "git_status": command(["git", "status", "--porcelain=v1"]),
                  "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                  "rustc": command(["rustc", "-Vv"]), "cargo": command(["cargo", "-V"]),
                  "expected_build": "release; workspace thin LTO; locked dependencies; host target",
                  "source_hashes": source_hashes(args.source_root.resolve(strict=True)),
                  "environment": environment(), "environment_note": args.environment_note,
                  "fonts": [], "workloads": []}
        for flag, path in (("--font", args.font), ("--emoji-font", args.emoji_font)):
            if path:
                report["fonts"].append({"role": flag[2:], "filename": path.name,
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        try:
            with tempfile.TemporaryDirectory(prefix="rusteditor-render-") as directory:
                for name, data, metadata in fixtures():
                    path = Path(directory) / (name + ".txt")
                    path.write_bytes(data)
                    cmd = [str(binary), str(path), str(args.samples)]
                    for flag, font in (("--font", args.font), ("--emoji-font", args.emoji_font)):
                        if font:
                            cmd.extend([flag, str(font.resolve(strict=True))])
                    print(f"{args.label}: {name}, {len(data)} bytes", flush=True)
                    result = json.loads(command(cmd))
                    if result["build"]["debug_assertions"]:
                        raise ValueError("diagnostic binary has debug_assertions; build with --release")
                    for scenario in result["scenarios"]:
                        scenario["summary"] = summarize(scenario["samples"])
                    report["workloads"].append({"name": name, "fixture": metadata, "result": result})
            report["status"] = "complete"
        except Exception as error:
            report["status"] = "failed"
            report["failure"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            report["finished_utc"] = datetime.now(timezone.utc).isoformat()
            stream.seek(0)
            json.dump(report, stream, indent=2, ensure_ascii=True)
            stream.write("\n")
            stream.truncate()
    print(args.output)


if __name__ == "__main__":
    main()
