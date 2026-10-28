"""Regression checks for diagnostic workload identity and statistical labeling."""
import hashlib
import unittest

from render_diagnostics import fixtures, summarize


class RenderDiagnosticTests(unittest.TestCase):
    def test_diagnostics_reuse_exact_gui_workload_bytes(self):
        workloads = {name: (data, metadata) for name, data, metadata in fixtures()}
        self.assertEqual(workloads["empty"][0], b"")
        self.assertEqual(len(workloads["lf"][0]), 1048576)
        self.assertEqual(hashlib.sha256(workloads["lf"][0]).hexdigest(),
                         "beea9f851adc1373c301a1d9609bd7815565f221eb943921f26e3c8ad1967f31")
        normalized, metadata = workloads["crlf-normalized"]
        self.assertEqual(len(normalized), 1028016)
        self.assertNotIn(b"\r", normalized)
        self.assertEqual(metadata["after_crlf_to_lf"]["sha256"], metadata["measured_sha256"])
        self.assertNotIn(b"\n", workloads["long-line"][0])

    def test_statistics_use_nearest_rank_without_discarding_outliers(self):
        metrics = ("cpu_ns", "allocated_bytes", "allocation_calls",
                   "deallocated_bytes", "deallocation_calls", "live_bytes_before",
                   "live_bytes_after", "peak_live_bytes", "peak_extra_live_bytes")
        samples = [dict.fromkeys(metrics, value) for value in range(20, 0, -1)]
        result = summarize(samples)
        for summary in result.values():
            self.assertEqual(summary, {"p50": 10, "p95": 19, "min": 1, "max": 20})
        self.assertEqual(samples[0]["cpu_ns"], 20)


if __name__ == "__main__":
    unittest.main()
