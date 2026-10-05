#!/usr/bin/env python3
import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("perf_gate.py")
SPEC = importlib.util.spec_from_file_location("perf_gate", MODULE_PATH)
assert SPEC and SPEC.loader
perf_gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(perf_gate)


class PerfGateTests(unittest.TestCase):
    def test_parses_criterion_output_and_uses_interval_median(self):
        parsed = perf_gate.parse_samples(
            "pipeline/full_diagnostics_large_valid\n"
            "                        time:   [1.0000 ms 1.2000 ms 1.4000 ms]\n"
        )
        self.assertEqual(parsed, {"pipeline/full_diagnostics_large_valid": [1_200_000.0]})

    def test_uses_median_of_repeated_samples(self):
        parsed = perf_gate.parse_samples("bench 1 ms\nbench 100 ms\nbench 2 ms\n")
        self.assertEqual(parsed["bench"], [1_000_000.0, 100_000_000.0, 2_000_000.0])
        row = perf_gate.compare(parsed, {"bench": [2_050_000.0]}, 1.05)[0]
        self.assertAlmostEqual(row[3], 1.025)
        self.assertTrue(row[4])

    def test_rejects_ratio_above_limit(self):
        row = perf_gate.compare({"bench": [100.0]}, {"bench": [105.01]}, 1.05)[0]
        self.assertFalse(row[4])

    def test_rejects_missing_head_benchmark(self):
        with self.assertRaisesRegex(ValueError, "missing benchmark"):
            perf_gate.compare({"bench": [100.0]}, {}, 1.05)


if __name__ == "__main__":
    unittest.main()
