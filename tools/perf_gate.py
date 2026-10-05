#!/usr/bin/env python3
"""Compare frontend benchmark medians and enforce a maximum slowdown ratio."""

from __future__ import annotations

import argparse
import re
import statistics
import subprocess
import sys
from pathlib import Path

ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")
VALUE_RE = re.compile(r"([0-9]+(?:\.[0-9]+)?)\s*(ns|us|µs|ms|s)\b")
SIMPLE_RE = re.compile(
    r"^([A-Za-z0-9_./:-]+)\s+([0-9]+(?:\.[0-9]+)?)\s*(ns|us|µs|ms|s)\s*$"
)
SCALE_NS = {"ns": 1.0, "us": 1_000.0, "µs": 1_000.0, "ms": 1_000_000.0, "s": 1_000_000_000.0}


def parse_samples(text: str) -> dict[str, list[float]]:
    """Parse Criterion output or ``benchmark value unit`` sample lines."""
    samples: dict[str, list[float]] = {}
    benchmark: str | None = None
    for raw_line in ANSI_RE.sub("", text).splitlines():
        line = raw_line.strip()
        simple = SIMPLE_RE.fullmatch(line)
        if simple:
            name, value, unit = simple.groups()
            samples.setdefault(name, []).append(float(value) * SCALE_NS[unit])
            continue
        if line and not line.startswith(("time:", "change:", "Found ")):
            # Criterion prints the benchmark name on the line before `time:`.
            if re.fullmatch(r"[A-Za-z0-9_./:-]+", line):
                benchmark = line
            continue
        if line.startswith("time:") and benchmark:
            values = VALUE_RE.findall(line)
            if values:
                # Criterion's interval is [lower median upper].
                value, unit = values[len(values) // 2]
                samples.setdefault(benchmark, []).append(float(value) * SCALE_NS[unit])
    return samples


def read_source(path: str | None, command: str | None, label: str) -> str:
    if path:
        return Path(path).read_text(encoding="utf-8")
    assert command is not None
    result = subprocess.run(command, shell=True, text=True, capture_output=True)
    if result.returncode:
        print(result.stdout, end="", file=sys.stderr)
        print(result.stderr, end="", file=sys.stderr)
        raise RuntimeError(f"{label} command failed with exit code {result.returncode}")
    return result.stdout + result.stderr


def compare(
    baseline: dict[str, list[float]], head: dict[str, list[float]], max_ratio: float
) -> list[tuple[str, float, float, float, bool]]:
    if not baseline:
        raise ValueError("baseline contains no benchmark samples")
    missing = sorted(set(baseline) - set(head))
    if missing:
        raise ValueError(f"head is missing benchmark(s): {', '.join(missing)}")
    rows = []
    for name in sorted(baseline):
        baseline_median = statistics.median(baseline[name])
        head_median = statistics.median(head[name])
        ratio = head_median / baseline_median
        rows.append((name, baseline_median, head_median, ratio, ratio <= max_ratio))
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    baseline = parser.add_mutually_exclusive_group(required=True)
    baseline.add_argument("--baseline", metavar="FILE")
    baseline.add_argument("--baseline-command", metavar="COMMAND")
    head = parser.add_mutually_exclusive_group(required=True)
    head.add_argument("--head", metavar="FILE")
    head.add_argument("--head-command", metavar="COMMAND")
    parser.add_argument("--max-ratio", type=float, default=1.05)
    args = parser.parse_args()
    if args.max_ratio <= 0:
        parser.error("--max-ratio must be positive")

    try:
        baseline_samples = parse_samples(
            read_source(args.baseline, args.baseline_command, "baseline")
        )
        head_samples = parse_samples(read_source(args.head, args.head_command, "head"))
        rows = compare(baseline_samples, head_samples, args.max_ratio)
    except (OSError, RuntimeError, ValueError) as error:
        print(f"PERF FAIL: {error}", file=sys.stderr)
        return 2

    failed = False
    for name, baseline_median, head_median, ratio, passed in rows:
        status = "PASS" if passed else "FAIL"
        print(
            f"{status} {name}: baseline={baseline_median / 1_000_000:.6f} ms "
            f"head={head_median / 1_000_000:.6f} ms ratio={ratio:.4f} "
            f"limit={args.max_ratio:.4f}"
        )
        failed |= not passed
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
