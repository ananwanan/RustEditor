"""Generate bounded, deterministic UTF-8 GUI latency fixtures without text I/O conversion."""

import argparse
import hashlib
import json
from pathlib import Path


MAX_BYTES = 4 * 1024 * 1024
UNIT = "hello \u4f60\u597d e\u0301 \U0001f469\U0001f3fd\u200d\U0001f4bb \U0001f1e8\U0001f1f3 1\ufe0f\u20e3"
ENDINGS = {"lf": "\n", "crlf": "\r\n", "long-line": ""}


def make_workload(variant: str, size: int) -> bytes:
    """Repeat complete Unicode units, then fill remaining bytes with ASCII a."""
    if variant not in ENDINGS:
        raise ValueError("unknown workload variant")
    if not 1 <= size <= MAX_BYTES:
        raise ValueError("size must be between 1 byte and the M0 4 MiB limit")
    unit = (UNIT + ENDINGS[variant]).encode("utf-8")
    repetitions, remainder = divmod(size, len(unit))
    return unit * repetitions + b"a" * remainder


def describe(data: bytes, variant: str) -> dict:
    """Include CRLF normalization metadata separately from the original fixture."""
    normalized = data.replace(b"\r\n", b"\n")
    unit_bytes = len((UNIT + ENDINGS[variant]).encode("utf-8"))
    return {
        "variant": variant,
        "utf8_bytes": len(data),
        "unicode_scalars": len(data.decode("utf-8")),
        "sha256": hashlib.sha256(data).hexdigest(),
        "unit_utf8_bytes": unit_bytes,
        "complete_units": len(data) // unit_bytes,
        "ascii_suffix_bytes": len(data) % unit_bytes,
        "linefeed_count": data.count(b"\n"),
        "carriage_return_count": data.count(b"\r"),
        "after_crlf_to_lf": {
            "utf8_bytes": len(normalized),
            "sha256": hashlib.sha256(normalized).hexdigest(),
        },
    }


def write_workload(output: Path, data: bytes) -> None:
    """Exclusive binary creation preserves fixtures and exact newline bytes."""
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("xb") as stream:
        stream.write(data)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", choices=ENDINGS, default="lf")
    parser.add_argument("--bytes", type=int, default=1024 * 1024, dest="size")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        data = make_workload(args.variant, args.size)
        write_workload(args.output, data)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(describe(data, args.variant), indent=2))


if __name__ == "__main__":
    main()
