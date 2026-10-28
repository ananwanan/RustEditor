"""Byte and newline invariants for the standalone GUI performance fixtures."""

import tempfile
import unittest
from pathlib import Path

from workload import ENDINGS, MAX_BYTES, UNIT, describe, make_workload, write_workload


class WorkloadTests(unittest.TestCase):
    def test_exact_size_and_utf8_with_byte_safe_suffix(self):
        for variant, ending in ENDINGS.items():
            unit = (UNIT + ending).encode("utf-8")
            for size in (1, len(unit) - 1, len(unit), len(unit) + 1, 1024 * 1024):
                with self.subTest(variant=variant, size=size):
                    data = make_workload(variant, size)
                    self.assertEqual(len(data), size)
                    self.assertEqual(data.decode("utf-8").encode("utf-8"), data)
                    count, remainder = divmod(size, len(unit))
                    self.assertEqual(data, unit * count + b"a" * remainder)

    def test_newline_variants_and_normalized_size_are_distinct(self):
        size = 1024 * 1024
        lf = make_workload("lf", size)
        crlf = make_workload("crlf", size)
        long_line = make_workload("long-line", size)
        self.assertNotIn(b"\r", lf)
        self.assertGreater(lf.count(b"\n"), 0)
        self.assertEqual(crlf.count(b"\r\n"), crlf.count(b"\n"))
        self.assertNotIn(b"\r", long_line)
        self.assertNotIn(b"\n", long_line)
        self.assertEqual(describe(lf, "lf")["after_crlf_to_lf"]["utf8_bytes"], size)
        self.assertEqual(
            describe(crlf, "crlf")["after_crlf_to_lf"]["utf8_bytes"],
            size - crlf.count(b"\r\n"),
        )
        self.assertNotEqual(crlf.replace(b"\r\n", b"\n"), lf)

    def test_invalid_requests_are_bounded(self):
        for size in (0, -1, MAX_BYTES + 1):
            with self.assertRaises(ValueError):
                make_workload("lf", size)
        with self.assertRaises(ValueError):
            make_workload("unknown", 1)

    def test_binary_creation_never_overwrites(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "fixture.txt"
            data = make_workload("crlf", 1024)
            write_workload(output, data)
            self.assertEqual(output.read_bytes(), data)
            with self.assertRaises(FileExistsError):
                write_workload(output, b"replacement")
            self.assertEqual(output.read_bytes(), data)


if __name__ == "__main__":
    unittest.main()
