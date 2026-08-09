import importlib.util
import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path


SCRIPT = Path(__file__).with_name("measure-desktop-memory.py")
SPEC = importlib.util.spec_from_file_location("measure_desktop_memory", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class MeasureDesktopMemoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name)
        self.proc = root / "proc"
        self.cgroup = root / "cgroup"
        self.registry = root / "registry"
        service = self.cgroup / "system.slice" / "wrdp.service"
        service.mkdir(parents=True)
        boot = self.proc / "sys/kernel/random"
        boot.mkdir(parents=True)
        (boot / "boot_id").write_text("test-boot-id\n", encoding="ascii")
        (service / "memory.current").write_text("4096\n", encoding="ascii")
        (service / "memory.peak").write_text("8192\n", encoding="ascii")
        (service / "cgroup.procs").write_text("10\n20\n", encoding="ascii")

        state_dir = self.registry / "1000"
        state_dir.mkdir(parents=True)
        (state_dir / "default.state.json").write_text(
            json.dumps(
                {
                    "requested_size": {"width": 1920, "height": 1080},
                    "active_clients": 1,
                    "components": [
                        {
                            "name": "compositor",
                            "pid": 20,
                            "start_ticks": 2020,
                            "boot_id": "test-boot-id",
                            "uid": 1000,
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )

        self.make_process(10, "wrdp", 0, 1, 10, 11, b"/usr/bin/wrdp\0--password\0server-secret\0")
        self.make_process(
            20,
            "wrdp-comp",
            1000,
            1,
            20,
            18,
            b"/usr/bin/wrdp-compositor\0--token=session-secret\0",
            b"XDG_RUNTIME_DIR=/run/user/1000/wrdp\0PRIVATE=do-not-report\0",
        )
        self.make_process(21, "terminal", 1000, 20, 30, 24, b"terminal\0--title\0secret-title\0")
        self.make_process(
            30,
            "portal",
            1000,
            1,
            40,
            35,
            b"portal\0secret-argument\0",
            b"XDG_RUNTIME_DIR=/run/user/1000/wrdp\0TOKEN=environment-secret\0",
        )
        self.make_process(
            40,
            "unrelated",
            1000,
            1,
            100,
            90,
            b"unrelated\0",
            b"XDG_RUNTIME_DIR=/run/user/1000/wrdp-extra\0",
        )

    def tearDown(self):
        self.temporary.cleanup()

    def make_process(self, pid, comm, uid, ppid, rss, pss, cmdline, environ=b""):
        directory = self.proc / str(pid)
        directory.mkdir(parents=True)
        (directory / "comm").write_text(f"{comm}\n", encoding="utf-8")
        (directory / "cmdline").write_bytes(cmdline)
        (directory / "environ").write_bytes(environ)
        stat_fields = ["S", str(ppid)] + ["0"] * 17 + [str(pid * 101)]
        if pid == 20:
            stat_fields[-1] = "2020"
        (directory / "stat").write_text(
            f"{pid} ({comm}) {' '.join(stat_fields)}\n", encoding="ascii"
        )
        (directory / "status").write_text(
            f"Name:\t{comm}\nPPid:\t{ppid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nVmRSS:\t{rss} kB\n",
            encoding="ascii",
        )
        (directory / "smaps_rollup").write_text(
            "\n".join(
                [
                    f"Rss: {rss} kB",
                    f"Pss: {pss} kB",
                    "Private_Clean: 1 kB",
                    "Private_Dirty: 2 kB",
                    "Shared_Clean: 3 kB",
                    "Shared_Dirty: 4 kB",
                ]
            )
            + "\n",
            encoding="ascii",
        )

    def arguments(self, samples="1"):
        return [
            "--label",
            "desktop-test",
            "--samples",
            samples,
            "--interval",
            "0",
            "--session-uid",
            "1000",
            "--proc-root",
            str(self.proc),
            "--cgroup-root",
            str(self.cgroup),
            "--registry-root",
            str(self.registry),
        ]

    def test_collects_union_redacts_and_summarizes(self):
        args = MODULE.parse_args(self.arguments(samples="2"))
        report = MODULE.run(args)

        self.assertEqual(report["label"], "desktop-test")
        self.assertEqual(report["scenario"]["requested_size"], {"width": 1920, "height": 1080})
        self.assertEqual(report["scenario"]["active_clients"], 1)
        sample = report["samples"][0]
        self.assertEqual([process["pid"] for process in sample["processes"]], [10, 20, 21, 30])
        by_pid = {process["pid"]: process for process in sample["processes"]}
        self.assertEqual(by_pid[10]["sources"], ["cgroup"])
        self.assertIn("state_component", by_pid[20]["sources"])
        self.assertEqual(by_pid[21]["sources"], ["state_descendant"])
        self.assertEqual(by_pid[30]["sources"], ["session_environment"])
        self.assertEqual(by_pid[20]["uid"], 1000)
        self.assertEqual(by_pid[20]["ppid"], 1)
        self.assertEqual(by_pid[20]["smaps_rollup"]["private_dirty_kib"], 2)
        self.assertEqual(sample["process_totals"]["rss_kib"], 100)
        self.assertEqual(sample["process_totals"]["pss_kib"], 88)

        summary = report["summary"]
        self.assertEqual(summary["cgroup"]["memory_current_bytes"]["mean"], 4096.0)
        self.assertEqual(summary["cgroup"]["memory_current_bytes"]["stdev"], 0.0)
        self.assertEqual(summary["total_unique_process_memory"]["rss_kib"]["mean"], 100.0)
        self.assertEqual(summary["per_comm"]["wrdp-comp"]["mean_process_count"], 1.0)

        rendered = json.dumps(report)
        for secret in ("server-secret", "session-secret", "secret-title", "environment-secret", "do-not-report"):
            self.assertNotIn(secret, rendered)
        self.assertEqual(by_pid[10]["cmdline"]["executable"], "wrdp")
        self.assertEqual(by_pid[10]["cmdline"]["argument_count"], 2)
        self.assertTrue(by_pid[10]["cmdline"]["arguments_redacted"])

    def test_embeds_bounded_evidence_object(self):
        evidence = Path(self.temporary.name) / "evidence.json"
        evidence.write_text(
            json.dumps({"kind": "reconnect", "pre_component": {"pid": 20}}),
            encoding="utf-8",
        )
        args = MODULE.parse_args(self.arguments() + ["--evidence-json", str(evidence)])
        report = MODULE.run(args)
        self.assertEqual(report["evidence"]["kind"], "reconnect")
        self.assertEqual(report["evidence"]["pre_component"]["pid"], 20)

    def test_missing_optional_files_and_output_modes(self):
        (self.cgroup / "system.slice" / "wrdp.service" / "memory.peak").unlink()
        (self.proc / "20" / "smaps_rollup").unlink()
        args = MODULE.parse_args(self.arguments())
        report = MODULE.run(args)

        sample = report["samples"][0]
        self.assertIsNone(sample["cgroup"]["memory_peak_bytes"])
        self.assertIn("cgroup.memory.peak: not_found", sample["errors"])
        process = next(item for item in sample["processes"] if item["pid"] == 20)
        self.assertIsNone(process["smaps_rollup"]["pss_kib"])
        self.assertIn("smaps_rollup: not_found", process["errors"])
        self.assertEqual(process["vmrss_kib"], 20)
        self.assertIsNone(sample["process_totals"]["pss_kib"])
        self.assertFalse(sample["process_totals"]["pss_complete"])

        stdout = io.StringIO()
        with redirect_stdout(stdout):
            return_code = MODULE.main(self.arguments())
        self.assertEqual(return_code, 0)
        self.assertEqual(json.loads(stdout.getvalue())["label"], "desktop-test")

        output = Path(self.temporary.name) / "report.json"
        output_args = self.arguments() + ["--output", str(output)]
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            return_code = MODULE.main(output_args)
        self.assertEqual(return_code, 0)
        self.assertEqual(stdout.getvalue(), "")
        self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["schema_version"], 1)

    def test_rejects_stale_component_identity(self):
        state_path = self.registry / "1000" / "default.state.json"
        state = json.loads(state_path.read_text(encoding="utf-8"))
        state["components"][0]["start_ticks"] = 9999
        state_path.write_text(json.dumps(state), encoding="utf-8")
        (self.cgroup / "system.slice/wrdp.service/cgroup.procs").write_text("10\n", encoding="ascii")
        args = MODULE.parse_args(self.arguments())
        report = MODULE.run(args)
        by_pid = {item["pid"]: item for item in report["samples"][0]["processes"]}
        self.assertIn(20, by_pid)
        self.assertEqual(by_pid[20]["sources"], ["session_environment"])
        self.assertNotIn(21, by_pid)

    def test_discovers_uid_and_requires_exact_runtime_directory(self):
        args = MODULE.parse_args(
            [
                "--label",
                "auto",
                "--samples",
                "1",
                "--interval",
                "0",
                "--proc-root",
                str(self.proc),
                "--cgroup-root",
                str(self.cgroup),
                "--registry-root",
                str(self.registry),
            ]
        )
        report = MODULE.run(args)
        pids = [process["pid"] for process in report["samples"][0]["processes"]]
        self.assertIn(30, pids)
        self.assertNotIn(40, pids)
        self.assertEqual(report["scenario"]["session_uid"], 1000)


if __name__ == "__main__":
    unittest.main()
