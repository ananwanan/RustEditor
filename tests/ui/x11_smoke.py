"""Exercise a real desktop on an isolated X11 display with xdotool and xclip.

Build with cargo xtask stage first, then run under xvfb-run on Linux. This is
not Windows IME/DPI acceptance. No Python packages or bundled fonts are needed.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def run():
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=root / "target/debug/rusteditor-desktop")
    parser.add_argument("--no-plugins", action="store_true")
    args = parser.parse_args()
    if not os.environ.get("DISPLAY"):
        parser.error("use an isolated X11 display, for example xvfb-run -a")
    if not args.binary.is_file():
        parser.error("desktop binary missing; run cargo xtask stage or supply --binary")
    for tool in ("xdotool", "xclip"):
        if not shutil.which(tool):
            parser.error(f"{tool} must be installed and available on PATH")

    def command(argv, **kwargs):
        return subprocess.check_output(argv, timeout=10, **kwargs)

    def xdo(*argv):
        return command(["xdotool", *argv], text=True).strip()

    def keys(*argv):
        xdo("key", "--clearmodifiers", *argv)

    def clipboard(text):
        subprocess.run(["xclip", "-selection", "clipboard"], input=text.encode(), check=True, timeout=10)

    def read_clipboard():
        return command(["xclip", "-selection", "clipboard", "-o"]).decode()

    def wait_for_clipboard_change(marker):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            actual = read_clipboard()
            if actual != marker:
                return actual
            time.sleep(0.02)
        raise AssertionError("desktop did not publish clipboard text")

    counter = 0

    def marker():
        nonlocal counter
        counter += 1
        value = f"rusteditor-ui-smoke-{os.getpid()}-{counter}"
        clipboard(value)
        return value

    def assert_editor(expected):
        # Clear the source clipboard before copying back. Otherwise an unchanged
        # paste source could falsely pass even if the desktop never copied text.
        value = marker()
        keys("ctrl+a", "ctrl+c")
        actual = wait_for_clipboard_change(value)
        assert actual == expected, (actual, expected)

    def paste(text):
        clipboard(text)
        keys("ctrl+a", "ctrl+v")
        # Let the native clipboard request finish before changing its owner.
        time.sleep(0.15)

    with tempfile.TemporaryFile(mode="w+") as log:
        app = subprocess.Popen([str(args.binary.resolve()), *(["--no-plugins"] if args.no_plugins else [])], cwd=root, stdout=log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if app.poll() is not None:
                    log.seek(0)
                    raise AssertionError("desktop exited: " + log.read())
                try:
                    window = xdo("search", "--onlyvisible", "--pid", str(app.pid), "--name", "RustEditor").splitlines()[0]
                    break
                except (subprocess.CalledProcessError, IndexError):
                    time.sleep(0.05)
            else:
                raise AssertionError("desktop window did not open")
            xdo("windowfocus", window)
            time.sleep(0.2)
            sample = "hello 你好 straße🙂 e\u0301\nsecond line"
            paste(sample)
            assert_editor(sample)
            if not args.no_plugins:
                keys("ctrl+shift+u")
                assert_editor(sample.upper())
                keys("ctrl+z")
                assert_editor(sample)
                keys("ctrl+y")
                assert_editor(sample.upper())
            for source_cluster in ("e\u0301", "👩🏽‍💻", "🇨🇳", "1️⃣", "\r\n"):
                # egui-winit normalizes CRLF in pasted clipboard text to LF.
                # Direct CRLF transactions are covered by the view/core tests.
                cluster = source_cluster.replace("\r\n", "\n")
                sample = "L" + cluster + "R"
                paste("L" + source_cluster + "R")
                assert_editor(sample)
                keys("ctrl+End", "Left", "BackSpace")
                assert_editor("LR")
                keys("ctrl+z")
                assert_editor(sample)
                keys("ctrl+Home", "Right", "Delete")
                assert_editor("LR")
                keys("ctrl+z", "ctrl+Home", "Right", "shift+Right")
                value = marker()
                keys("ctrl+x")
                assert wait_for_clipboard_change(value) == cluster
                assert_editor("LR")
            print("[M0:UNICODE:UI:OK] native X11 clipboard, grapheme movement/deletion/cut, undo/redo; plugins=" + str(not args.no_plugins))
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait(timeout=5)


if __name__ == "__main__":
    run()
