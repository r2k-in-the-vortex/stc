"""Unit tests for the WL-4 campaign harness.

Run with:
    python3 -m unittest discover -s tools/rt/tests -v

The tests use only the standard library and do not need root, a launcher, or a
real control socket.
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import campaign


def metrics_header(**overrides: object) -> dict:
    header = {
        "record": "header",
        "protocol": campaign.METRICS_PROTOCOL,
        "build_id": "0xabc",
        "cycle_time_ns": 1_000_000,
        "priority": 90,
        "cpu": 3,
        "pid": 1234,
        "started_monotonic_ns": 0,
        "fault_interval": 1000,
    }
    header.update(overrides)
    return header


def metrics_cycle(cycle: int, wake: int = 100, execution: int = 1000, **overrides: object) -> dict:
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


def metrics_summary(cycles: int) -> dict:
    return {
        "record": "summary",
        "cycles": cycles,
        "missed_deadlines": 0,
        "skipped_periods": 0,
        "max_wake_lateness_ns": 0,
        "max_execution_ns": 0,
        "max_scan_overrun_ns": 0,
        "last_minor_faults": None,
        "last_major_faults": None,
        "last_allocations": None,
        "last_frees": None,
        "finished_monotonic_ns": 0,
    }


class DurationTests(unittest.TestCase):
    def test_units_and_bare_seconds(self) -> None:
        self.assertEqual(campaign.parse_duration_ns("10ns"), 10)
        self.assertEqual(campaign.parse_duration_ns("2us"), 2_000)
        self.assertEqual(campaign.parse_duration_ns("2µs"), 2_000)
        self.assertEqual(campaign.parse_duration_ns("5ms"), 5_000_000)
        self.assertEqual(campaign.parse_duration_ns("3"), 3_000_000_000)
        self.assertEqual(campaign.parse_duration_ns("1s"), 1_000_000_000)
        self.assertEqual(campaign.parse_duration_ns("2m"), 120_000_000_000)
        self.assertEqual(campaign.parse_duration_ns("1h"), 3_600_000_000_000)

    def test_invalid(self) -> None:
        for value in ("", "ms", "-1s", "1xs", "1.5s"):
            with self.assertRaises(ValueError):
                campaign.parse_duration_ns(value)


class StatisticsTests(unittest.TestCase):
    def test_nearest_rank(self) -> None:
        values = [10, 20, 30, 40, 50]
        self.assertEqual(campaign.nearest_rank(values, 0), 10)
        self.assertEqual(campaign.nearest_rank(values, 50), 30)
        self.assertEqual(campaign.nearest_rank(values, 99), 50)
        self.assertIsNone(campaign.nearest_rank([], 50))

    def test_stats_block(self) -> None:
        block = campaign.stats([1, 2, 3, 4])
        self.assertEqual(block["count"], 4)
        self.assertEqual(block["min"], 1)
        self.assertEqual(block["max"], 4)
        self.assertIn("p99_9", block)
        empty = campaign.stats([])
        self.assertEqual(empty["count"], 0)
        self.assertIsNone(empty["p50"])


class ControlWireTests(unittest.TestCase):
    def test_body_and_redaction(self) -> None:
        body = campaign.control_body(
            "secret", 7, "deploy", "/tmp/a.so", "/tmp/a.json"
        )
        self.assertEqual(body["protocol"], 1)
        self.assertEqual(body["nonce"], "secret")
        self.assertEqual(body["id"], 7)
        self.assertEqual(body["module"], "/tmp/a.so")
        self.assertEqual(body["manifest"], "/tmp/a.json")
        redacted = campaign.redact_request(body)
        self.assertNotIn("nonce", redacted)
        self.assertEqual(redacted["op"], "deploy")

    def test_status_body_has_no_paths(self) -> None:
        body = campaign.control_body("secret", 1, "status")
        self.assertNotIn("module", body)
        self.assertNotIn("manifest", body)

    def test_response_validation(self) -> None:
        response = {"protocol": 1, "id": 3, "ok": True}
        self.assertEqual(campaign.validate_control_response(response, 3), response)
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_control_response({"protocol": 2, "id": 3}, 3)
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_control_response({"protocol": 1, "id": 4}, 3)
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_control_response([], 1)


class ControlModeTests(unittest.TestCase):
    def workload(self) -> dict:
        return {"launcher": Path("/tmp/rt-bench")}

    def test_socket_launcher_command(self) -> None:
        command = campaign.launcher_command(
            self.workload(),
            1_000_000,
            90,
            3,
            Path("/tmp/cycles.jsonl"),
            "/tmp/control.sock",
            "nonce",
            "socket",
            0,
        )
        self.assertEqual(command[command.index("--cycles") + 1], "0")
        self.assertIn("--control", command)
        self.assertIn("--control-nonce", command)
        self.assertIn("nonce", command)

    def test_disabled_launcher_command(self) -> None:
        command = campaign.launcher_command(
            self.workload(),
            1_000_000,
            90,
            3,
            Path("/tmp/cycles.jsonl"),
            "",
            "",
            "none",
            12001,
        )
        self.assertEqual(command[command.index("--cycles") + 1], "12001")
        self.assertNotIn("--control", command)
        self.assertNotIn("--control-nonce", command)

    def test_steady_cycle_count(self) -> None:
        self.assertEqual(
            campaign.steady_cycle_count(2_000_000_000, 10_000_000_000, 1_000_000),
            12001,
        )

    def test_control_mode_validation(self) -> None:
        args = campaign.build_parser().parse_args(
            ["--out", "run", "--control-mode", "none", "--status-hz", "1"]
        )
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_args(args)
        args = campaign.build_parser().parse_args(
            ["--out", "run", "--mode", "online", "--control-mode", "none"]
        )
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_args(args)
        args = campaign.build_parser().parse_args(
            ["--out", "run", "--control-mode", "none"]
        )
        campaign.validate_args(args)


class MetricsTests(unittest.TestCase):
    def write_metrics(self, directory: Path, lines: list[dict]) -> Path:
        path = directory / "cycles.jsonl"
        with path.open("w", encoding="utf-8") as handle:
            for line in lines:
                handle.write(json.dumps(line, separators=(",", ":")) + "\n")
        return path

    def test_parse_and_stats(self) -> None:
        cycles = [
            metrics_cycle(1, wake=5, execution=100),
            metrics_cycle(
                2,
                wake=7,
                execution=200,
                missed_deadline=True,
                minor_faults=1,
                major_faults=0,
                allocations=2,
                frees=2,
            ),
            metrics_cycle(3, wake=9, execution=300),
            metrics_cycle(
                4,
                wake=11,
                execution=400,
                minor_faults=3,
                major_faults=1,
                allocations=4,
                frees=3,
            ),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_metrics(
                Path(temporary),
                [metrics_header(), *cycles, metrics_summary(4)],
            )
            parsed = campaign.parse_metrics(path)
        self.assertEqual(parsed["header"]["build_id"], "0xabc")
        self.assertEqual(len(parsed["cycles"]), 4)
        block = campaign.metrics_stats(parsed, measure_start_cycle=2, duration_ns=3_000_000, cycle_time_ns=1_000_000)
        self.assertEqual(block["measured_cycles"], 3)
        self.assertEqual(block["missed_deadlines"], 1)
        self.assertEqual(block["wake_lateness_ns"]["min"], 7)
        self.assertEqual(block["wake_lateness_ns"]["max"], 11)
        self.assertEqual(block["execution_ns"]["p50"], 300)
        self.assertEqual(block["minor_faults"]["delta"], 2)
        self.assertEqual(block["major_faults"]["delta"], 1)
        self.assertEqual(block["allocations"]["delta"], 2)
        self.assertEqual(block["frees"]["delta"], 1)
        self.assertEqual(block["expected_cycles"], 3)

    def test_protocol_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_metrics(
                Path(temporary),
                [metrics_header(protocol="other"), metrics_summary(0)],
            )
            with self.assertRaises(campaign.HarnessError):
                campaign.parse_metrics(path)

    def test_missing_header(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = self.write_metrics(
                Path(temporary), [metrics_cycle(1), metrics_summary(1)]
            )
            with self.assertRaises(campaign.HarnessError):
                campaign.parse_metrics(path)

    def test_generation_transition(self) -> None:
        cycles = [
            metrics_cycle(1, generation=1),
            metrics_cycle(2, generation=1),
            metrics_cycle(3, generation=2),
            metrics_cycle(4, generation=2),
            metrics_cycle(5, generation=1),
        ]
        transition = campaign.generation_transition(cycles, 2)
        self.assertEqual(transition["activation_cycle"], 3)
        self.assertEqual(transition["last_cycle"], 4)
        self.assertEqual(transition["rollback_cycle"], 5)
        self.assertEqual(transition["cycles_in_generation"], 2)
        absent = campaign.generation_transition(cycles, 9)
        self.assertIsNone(absent["activation_cycle"])


class BundleTests(unittest.TestCase):
    def test_manifest_build_id(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "m.json"
            path.write_text(
                json.dumps({"body": {"build_id": "0xfeed"}}), encoding="utf-8"
            )
            self.assertEqual(campaign.manifest_build_id(path), "0xfeed")
            path.write_text(
                json.dumps({"body": {"header": {"build_id": "0xbeef"}}}),
                encoding="utf-8",
            )
            self.assertEqual(campaign.manifest_build_id(path), "0xbeef")
            path.write_text("not json", encoding="utf-8")
            self.assertIsNone(campaign.manifest_build_id(path))

    def test_resolve_candidate_single(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "bundle" / "candidates" / "code-only"
            directory.mkdir(parents=True)
            (directory / "rt-bench-abc.so").write_bytes(b"module")
            (directory / "rt-bench.stc-got.json").write_text(
                json.dumps({"body": {"header": {"build_id": "0x1"}}}),
                encoding="utf-8",
            )
            (directory / "rt-bench.stc-got.bin").write_bytes(b"binary")
            candidate = campaign.resolve_candidate(Path(temporary) / "bundle", "code-only")
        self.assertEqual(candidate["build_id"], "0x1")
        self.assertTrue(candidate["module"].name.endswith(".so"))
        self.assertTrue(candidate["manifest"].name.endswith(".json"))
        self.assertTrue(candidate["manifest_bin"].name.endswith(".bin"))

    def test_resolve_candidate_duplicate(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "bundle" / "candidates" / "code-only"
            directory.mkdir(parents=True)
            (directory / "rt-bench-a.so").write_bytes(b"a")
            (directory / "rt-bench-b.so").write_bytes(b"b")
            (directory / "rt-bench.stc-got.json").write_text("{}", encoding="utf-8")
            (directory / "rt-bench.stc-got.bin").write_bytes(b"binary")
            with self.assertRaises(campaign.HarnessError):
                campaign.resolve_candidate(Path(temporary) / "bundle", "code-only")


class LoadTests(unittest.TestCase):
    def test_none(self) -> None:
        self.assertIsNone(campaign.load_command("none", [0], 10))

    def test_cpu(self) -> None:
        command = campaign.load_command("cpu", [0, 1], 10)
        self.assertIsNotNone(command)
        assert command is not None
        self.assertEqual(command[0], "/usr/bin/taskset")
        self.assertIn("--cpu", command)
        self.assertNotIn("--vm", command)

    def test_mixed(self) -> None:
        command = campaign.load_command("mixed", [3], 10)
        self.assertIsNotNone(command)
        assert command is not None
        self.assertIn("--vm", command)
        self.assertIn("--hdd", command)
        self.assertIn("--sock", command)

    def test_empty_cpus(self) -> None:
        with self.assertRaises(campaign.HarnessError):
            campaign.load_command("cpu", [], 10)

    def test_load_cpus(self) -> None:
        with mock.patch.object(campaign.os, "cpu_count", return_value=4):
            self.assertEqual(campaign._load_cpus("r1", "none", 3), [])
            self.assertEqual(campaign._load_cpus("r1", "cpu", 3), [0, 1, 2])
            self.assertEqual(campaign._load_cpus("r2", "cpu", 3), [3])


class SummaryTests(unittest.TestCase):
    def test_format_ns(self) -> None:
        self.assertEqual(campaign.format_ns(1_000), "1.000 us")
        self.assertEqual(campaign.format_ns(1_000_000), "1.000 ms")
        self.assertEqual(campaign.format_ns(-1_000), "-1.000 us")
        self.assertEqual(campaign.format_ns(None), "null")

    def test_render_steady(self) -> None:
        summary = {
            "schema": campaign.SUMMARY_SCHEMA,
            "run_id": "r0",
            "mode": "steady",
            "regime": "r0",
            "result": "passed",
            "workload": "base",
            "cycle_time_ns": 1_000_000,
            "duration_ns": 10_000_000,
            "warmup_ns": 2_000_000,
            "rt": {"priority": 90, "cpu": 3},
            "load_profile": "none",
            "bundle": {"build_id": "0xabc"},
            "metrics": {
                "measured_cycles": 8,
                "expected_cycles": 10,
                "missed_deadlines": 0,
                "skipped_periods": 0,
                "wake_lateness_ns": {
                    "min": 1,
                    "p50": 2,
                    "p90": 3,
                    "p99": 4,
                    "p99_9": 5,
                    "max": 6,
                },
                "execution_ns": {
                    "min": 10,
                    "p50": 20,
                    "p90": 30,
                    "p99": 40,
                    "p99_9": 50,
                    "max": 60,
                },
                "minor_faults": {"delta": 0},
                "major_faults": {"delta": 0},
                "allocations": {"delta": 0},
                "frees": {"delta": 0},
            },
            "errors": [],
        }
        text = campaign.render_markdown(summary)
        self.assertIn("Wake lateness", text)
        self.assertIn("Execution", text)
        self.assertIn("passed", text)

    def test_render_online(self) -> None:
        summary = {
            "schema": campaign.SUMMARY_SCHEMA,
            "run_id": "online",
            "mode": "online",
            "regime": "r0",
            "result": "passed",
            "cases": {
                "code-only": {
                    "result": "passed",
                    "candidate": {"base_build_id": "0x1", "build_id": "0x2"},
                    "deploy": {"response": {"ok": True}, "rollback": {"observed": False}},
                },
                "trapping": {
                    "result": "passed",
                    "candidate": {"base_build_id": "0x1", "build_id": "0x3"},
                    "deploy": {"response": {"ok": True}, "rollback": {"observed": True}},
                },
            },
            "errors": [],
        }
        text = campaign.render_markdown(summary)
        self.assertIn("code-only", text)
        self.assertIn("trapping", text)
        self.assertIn("Rollback", text)

    def test_summary_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary) / "run"
            out.mkdir()
            summary = {
                "schema": campaign.SUMMARY_SCHEMA,
                "run_id": "run",
                "mode": "steady",
                "result": "passed",
                "metrics": {},
                "errors": [],
            }
            campaign.write_summary_files(out, summary)
            self.assertTrue((out / "summary.json").is_file())
            self.assertTrue((out / "summary.md").is_file())
            data = json.loads((out / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(data["schema"], campaign.SUMMARY_SCHEMA)


class ArgumentTests(unittest.TestCase):
    def test_valid(self) -> None:
        args = campaign.build_parser().parse_args(["--out", "run"])
        campaign.validate_args(args)

    def test_priority(self) -> None:
        args = campaign.build_parser().parse_args(
            ["--out", "run", "--rt-priority", "0"]
        )
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_args(args)

    def test_duration(self) -> None:
        args = campaign.build_parser().parse_args(["--out", "run", "--duration", "0"])
        with self.assertRaises(campaign.HarnessError):
            campaign.validate_args(args)

    def test_online_case(self) -> None:
        args = campaign.build_parser().parse_args(
            ["--out", "run", "--mode", "online", "--online-case", "all"]
        )
        campaign.validate_args(args)


if __name__ == "__main__":
    unittest.main()
