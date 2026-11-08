"""Unit tests for the WL-14 scheduling comparison harness.

Run with:
    python3 -m unittest discover -s tools/rt/tests -v

The tests use only the standard library and do not need root, a real launcher,
or a control socket.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import sched_compare


def metrics_header(**overrides: object) -> dict:
    header = {
        "record": "header",
        "protocol": sched_compare.METRICS_PROTOCOL,
        "build_id": "0xabc",
        "cycle_time_ns": 1_000_000,
        "priority": 0,
        "sched": "mmcss",
        "cpu": None,
        "pid": 1234,
        "started_monotonic_ns": 0,
        "fault_interval": 1000,
    }
    header.update(overrides)
    return header


def metrics_cycle(
    cycle: int,
    wake: int = 100,
    execution: int = 1000,
    **overrides: object,
) -> dict:
    record = {
        "record": "cycle",
        "cycle": cycle,
        "generation": 1,
        "deadline_ns": cycle * 1_000_000,
        "wake_ns": cycle * 1_000_000 + wake,
        "scan_start_ns": cycle * 1_000_000 + wake,
        "scan_end_ns": cycle * 1_000_000 + wake + execution,
        "wake_lateness_ns": wake,
        "execution_ns": execution,
        "scan_overrun_ns": 0,
        "missed_deadline": False,
        "skipped_periods": 0,
        "minor_faults": None,
        "major_faults": None,
        "allocations": None,
        "frees": None,
    }
    record.update(overrides)
    return record


def metrics_summary(cycles: int, **overrides: object) -> dict:
    summary = {
        "record": "summary",
        "cycles": cycles,
        "missed_deadlines": 0,
        "skipped_periods": 0,
        "dropped_records": 0,
        "max_wake_lateness_ns": 0,
        "max_execution_ns": 0,
        "max_scan_overrun_ns": 0,
        "last_minor_faults": None,
        "last_major_faults": None,
        "last_allocations": None,
        "last_frees": None,
        "finished_monotonic_ns": 0,
    }
    summary.update(overrides)
    return summary


def write_metrics(path: Path, lines: list[dict]) -> None:
    with path.open("w", encoding="utf-8") as handle:
        for line in lines:
            handle.write(json.dumps(line, separators=(",", ":")) + "\n")


class DurationTests(unittest.TestCase):
    def test_units_and_bare_seconds(self) -> None:
        self.assertEqual(sched_compare.parse_duration_ns("10ns"), 10)
        self.assertEqual(sched_compare.parse_duration_ns("2us"), 2_000)
        self.assertEqual(sched_compare.parse_duration_ns("2µs"), 2_000)
        self.assertEqual(sched_compare.parse_duration_ns("5ms"), 5_000_000)
        self.assertEqual(sched_compare.parse_duration_ns("3"), 3_000_000_000)
        self.assertEqual(sched_compare.parse_duration_ns("1s"), 1_000_000_000)
        self.assertEqual(sched_compare.parse_duration_ns("2m"), 120_000_000_000)
        self.assertEqual(sched_compare.parse_duration_ns("1h"), 3_600_000_000_000)

    def test_invalid(self) -> None:
        for value in ("", "ms", "-1s", "1xs", "1.5s"):
            with self.assertRaises(ValueError):
                sched_compare.parse_duration_ns(value)


class StatisticsTests(unittest.TestCase):
    def test_nearest_rank(self) -> None:
        values = [10, 20, 30, 40, 50]
        self.assertEqual(sched_compare.nearest_rank(values, 0), 10)
        self.assertEqual(sched_compare.nearest_rank(values, 50), 30)
        self.assertEqual(sched_compare.nearest_rank(values, 99), 50)
        self.assertIsNone(sched_compare.nearest_rank([], 50))

    def test_stats_block(self) -> None:
        block = sched_compare.stats([1, 2, 3, 4])
        self.assertEqual(block["count"], 4)
        self.assertEqual(block["min"], 1)
        self.assertEqual(block["max"], 4)
        self.assertIn("p99_9", block)
        empty = sched_compare.stats([])
        self.assertEqual(empty["count"], 0)
        self.assertIsNone(empty["p50"])


class TimeoutTests(unittest.TestCase):
    def test_timeout_covers_slow_macos_sleep_granularity(self) -> None:
        # 20,000 cycles at 1 ms is 20 s nominal; the macOS default loop was
        # observed at about 4.5 ms/cycle, so the cap must exceed 90 s.
        self.assertGreaterEqual(
            sched_compare.run_timeout_s(20_000, 1_000_000), 200.0
        )

    def test_timeout_has_a_floor(self) -> None:
        self.assertGreaterEqual(sched_compare.run_timeout_s(1, 1_000_000), 120.0)


class CommandTests(unittest.TestCase):
    def launcher(self) -> Path:
        return Path("build") / "rt-bench"

    def test_baseline_command(self) -> None:
        command = sched_compare.launcher_command(
            self.launcher(), 1_000_000, 20_000, "baseline", None
        )
        self.assertEqual(command[command.index("--cycles") + 1], "20000")
        self.assertEqual(command[command.index("--cycle-time") + 1], "1000000ns")
        self.assertNotIn("--rt", command)
        self.assertNotIn("--rt-metrics", command)

    def test_rt_command(self) -> None:
        command = sched_compare.launcher_command(
            self.launcher(),
            1_000_000,
            20_000,
            "rt",
            Path("run.jsonl"),
        )
        self.assertIn("--rt", command)
        self.assertEqual(command[command.index("--rt-metrics") + 1], "run.jsonl")

    def test_rt_requires_metrics_path(self) -> None:
        with self.assertRaises(sched_compare.CompareError):
            sched_compare.launcher_command(
                self.launcher(), 1_000_000, 20_000, "rt", None
            )

    def test_unknown_mode(self) -> None:
        with self.assertRaises(sched_compare.CompareError):
            sched_compare.launcher_command(
                self.launcher(), 1_000_000, 20_000, "other", None
            )


class MetricsTests(unittest.TestCase):
    def test_parse_and_analyze(self) -> None:
        cycles = [
            metrics_cycle(1, wake=5, execution=100),
            metrics_cycle(2, wake=7, execution=200, missed_deadline=True),
            metrics_cycle(
                3,
                wake=9,
                execution=300,
                skipped_periods=1,
            ),
            metrics_cycle(4, wake=11, execution=400),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "cycles.jsonl"
            write_metrics(path, [metrics_header(), *cycles, metrics_summary(4)])
            parsed = sched_compare.parse_metrics(path)
        self.assertEqual(parsed["header"]["sched"], "mmcss")
        self.assertEqual(len(parsed["cycles"]), 4)
        block = sched_compare.analyze_metrics(parsed, warmup_cycles=2)
        self.assertEqual(block["cycles_in_file"], 4)
        self.assertEqual(block["measured_cycles"], 2)
        self.assertEqual(block["first_measured_cycle"], 3)
        self.assertEqual(block["missed_deadlines"], 0)
        self.assertEqual(block["skipped_periods"], 1)
        self.assertEqual(block["execution_ns"]["min"], 300)
        self.assertEqual(block["execution_ns"]["max"], 400)
        self.assertEqual(block["wake_lateness_ns"]["min"], 9)
        self.assertEqual(block["wake_lateness_ns"]["max"], 11)
        self.assertEqual(block["_values"]["execution_ns"], [300, 400])

    def test_warmup_discards_everything(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "cycles.jsonl"
            write_metrics(
                path,
                [metrics_header(), metrics_cycle(1), metrics_summary(1)],
            )
            parsed = sched_compare.parse_metrics(path)
        with self.assertRaises(sched_compare.CompareError):
            sched_compare.analyze_metrics(parsed, warmup_cycles=2000)

    def test_protocol_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "cycles.jsonl"
            write_metrics(
                path,
                [metrics_header(protocol="other"), metrics_summary(0)],
            )
            with self.assertRaises(sched_compare.CompareError):
                sched_compare.parse_metrics(path)

    def test_missing_header(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "cycles.jsonl"
            write_metrics(path, [metrics_cycle(1), metrics_summary(1)])
            with self.assertRaises(sched_compare.CompareError):
                sched_compare.parse_metrics(path)

    def test_unknown_record(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "cycles.jsonl"
            write_metrics(
                path,
                [metrics_header(), {"record": "other"}, metrics_summary(0)],
            )
            with self.assertRaises(sched_compare.CompareError):
                sched_compare.parse_metrics(path)

    def test_dropped_records_from_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "cycles.jsonl"
            write_metrics(
                path,
                [
                    metrics_header(),
                    metrics_cycle(1),
                    metrics_cycle(2),
                    metrics_summary(2, dropped_records=3),
                ],
            )
            parsed = sched_compare.parse_metrics(path)
        block = sched_compare.analyze_metrics(parsed, warmup_cycles=0)
        self.assertEqual(block["dropped_records"], 3)


class RunTests(unittest.TestCase):
    def launcher(self, directory: Path) -> Path:
        launcher = directory / "rt-bench"
        launcher.write_text("", encoding="utf-8")
        return launcher

    def test_baseline_run(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher = self.launcher(root)
            out = root / "out"
            completed = subprocess.CompletedProcess(["rt-bench"], 0, "", "")
            with mock.patch.object(
                sched_compare.subprocess, "run", return_value=completed
            ):
                record = sched_compare.run_once(
                    launcher, 1_000_000, 1, "baseline", 1, out, 0
                )
            self.assertIsNone(record["error"])
            self.assertEqual(record["return_code"], 0)
            self.assertIsNone(record["metrics_file"])
            self.assertIsNone(record["metrics"])
            self.assertGreater(record["wall_time_ns"], 0)
            self.assertGreater(record["wall_cycle_ns"]["mean"], 0)

    def test_rt_run_parses_metrics(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher = self.launcher(root)
            out = root / "out"

            def fake_run(command: list[str], **_: object) -> subprocess.CompletedProcess:
                metrics_path = Path(command[command.index("--rt-metrics") + 1])
                write_metrics(
                    metrics_path,
                    [
                        metrics_header(),
                        metrics_cycle(1),
                        metrics_cycle(2, wake=20, execution=200),
                        metrics_summary(2, dropped_records=1),
                    ],
                )
                return subprocess.CompletedProcess(command, 0, "", "")

            with mock.patch.object(
                sched_compare.subprocess, "run", side_effect=fake_run
            ):
                record = sched_compare.run_once(
                    launcher, 1_000_000, 2, "rt", 1, out, 1
                )
            self.assertIsNone(record["error"])
            self.assertEqual(record["header"]["sched"], "mmcss")
            self.assertEqual(record["metrics"]["measured_cycles"], 1)
            self.assertEqual(record["metrics"]["execution_ns"]["p50"], 200)
            self.assertEqual(record["metrics"]["dropped_records"], 1)
            self.assertIsNotNone(record["metrics_sha256"])

    def test_failure_is_recorded(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher = self.launcher(root)
            out = root / "out"
            completed = subprocess.CompletedProcess(["rt-bench"], 2, "", "bad")
            with mock.patch.object(
                sched_compare.subprocess, "run", return_value=completed
            ):
                record = sched_compare.run_once(
                    launcher, 1_000_000, 10, "baseline", 1, out, 2
                )
            self.assertIn("status 2", record["error"])


class CollectTests(unittest.TestCase):
    def test_summary_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            launcher = root / "rt-bench"
            launcher.write_text("", encoding="utf-8")
            out = root / "out"
            records = [
                {
                    "run": 1,
                    "mode": "rt",
                    "wall_time_ns": 2_000_000,
                    "wall_cycle_ns": {"mean": 1_000},
                    "return_code": 0,
                    "metrics": {
                        "measured_cycles": 2,
                        "missed_deadlines": 0,
                        "skipped_periods": 0,
                        "dropped_records": 0,
                        "execution_ns": sched_compare.stats([100, 200]),
                        "wake_lateness_ns": sched_compare.stats([10, 20]),
                        "_values": {
                            "execution_ns": [100, 200],
                            "wake_lateness_ns": [10, 20],
                        },
                    },
                    "header": {"sched": "time_constraint"},
                    "error": None,
                }
            ]
            with mock.patch.object(
                sched_compare, "collect_host_info", return_value={"hostname": "test"}
            ), mock.patch.object(
                sched_compare, "manifest_build_id", return_value=("0xabc", "m.json")
            ), mock.patch.object(
                sched_compare, "run_once", side_effect=records
            ):
                summary = sched_compare.collect_mode(
                    launcher, 1_000_000, 2, 1, "rt", out, 1, "debug"
                )
            self.assertEqual(summary["result"], "passed")
            self.assertEqual(summary["launcher"]["build_id"], "0xabc")
            self.assertNotIn("_values", summary["runs"][0]["metrics"])
            data = json.loads((out / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(data["schema"], sched_compare.SUMMARY_SCHEMA)
            self.assertEqual(data["aggregate"]["execution_ns"]["p50"], 100)
            text = (out / "summary.md").read_text(encoding="utf-8")
            self.assertIn("time_constraint", text)
            self.assertIn("Aggregate", text)

    def test_baseline_markdown_marks_metrics_unavailable(self) -> None:
        summary = {
            "schema": sched_compare.SUMMARY_SCHEMA,
            "generated_at": "2026-11-08T00:00:00+00:00",
            "mode": "baseline",
            "build_form": "debug",
            "launcher": {
                "path": "rt-bench",
                "sha256": "abc",
                "build_id": "0x1",
                "manifest": "m.json",
            },
            "host": {
                "hostname": "test",
                "platform": "Windows",
                "cpu_model": "CPU",
                "cpu_count": 4,
            },
            "parameters": {
                "cycle_time_ns": 1_000_000,
                "cycles_per_run": 10,
                "run_count": 1,
                "warmup_cycles": 2,
            },
            "runs": [
                {
                    "run": 1,
                    "return_code": 0,
                    "wall_time_ns": 10_000_000,
                    "metrics": None,
                    "header": None,
                }
            ],
            "aggregate": {
                "measured_cycles": 0,
                "missed_deadlines": None,
                "skipped_periods": None,
                "dropped_records": None,
                "execution_ns": sched_compare.stats([]),
                "wake_lateness_ns": sched_compare.stats([]),
                "wall_cycle_ns": sched_compare.stats([1_000_000]),
                "wall_time_ns": sched_compare.stats([10_000_000]),
            },
            "result": "passed",
            "errors": [],
            "notes": ["baseline has no per-cycle metrics"],
        }
        text = sched_compare.render_markdown(summary)
        self.assertIn("n/a", text)
        self.assertIn("baseline has no per-cycle metrics", text)
        self.assertIn("**passed**", text)


class HostTests(unittest.TestCase):
    def test_host_info_shape(self) -> None:
        with mock.patch.object(sched_compare, "cpu_model", return_value="CPU"), \
                mock.patch.object(sched_compare, "os_product", return_value={}):
            host = sched_compare.collect_host_info()
        self.assertEqual(host["schema"], sched_compare.HOST_SCHEMA)
        self.assertIn("platform", host)
        self.assertIn("machine", host)
        self.assertIn("cpu_count", host)


class ArgumentTests(unittest.TestCase):
    def test_valid(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            launcher = Path(temporary) / "rt-bench"
            launcher.write_text("", encoding="utf-8")
            args = sched_compare.build_parser().parse_args(
                ["--launcher", str(launcher), "--mode", "rt", "--out", "run"]
            )
            sched_compare.validate_args(args)

    def test_missing_launcher(self) -> None:
        args = sched_compare.build_parser().parse_args(
            ["--launcher", "missing", "--mode", "rt", "--out", "run"]
        )
        with self.assertRaises(sched_compare.CompareError):
            sched_compare.validate_args(args)

    def test_warmup_window(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            launcher = Path(temporary) / "rt-bench"
            launcher.write_text("", encoding="utf-8")
            args = sched_compare.build_parser().parse_args(
                [
                    "--launcher",
                    str(launcher),
                    "--mode",
                    "rt",
                    "--out",
                    "run",
                    "--cycles",
                    "10",
                    "--warmup-cycles",
                    "10",
                ]
            )
            with self.assertRaises(sched_compare.CompareError):
                sched_compare.validate_args(args)


if __name__ == "__main__":
    unittest.main()
