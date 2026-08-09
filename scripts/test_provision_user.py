import importlib.util
import os
import pwd
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("provision-user.py")
SPEC = importlib.util.spec_from_file_location("provision_user", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class ProvisionUserTests(unittest.TestCase):
    def test_rejects_invalid_account_name(self):
        with patch.object(MODULE.os, "geteuid", return_value=0), patch.dict(
            os.environ, {"PROVISION_USER": "bad'user", "WRDP_SOURCE_DIR": "/tmp"}, clear=True
        ), self.assertRaises(SystemExit):
            MODULE.main()

    def test_uses_absolute_runuser_and_target_home(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            runner = source / "vendor/ground-init/ground-init.py"
            profile = source / "ground-init.user.yaml"
            runner.parent.mkdir(parents=True)
            runner.write_text("", encoding="utf-8")
            profile.write_text("---\n", encoding="utf-8")
            account = SimpleNamespace(pw_dir="/home/tester")
            completed = SimpleNamespace(returncode=0)
            with patch.object(MODULE.os, "geteuid", return_value=0), patch.object(
                pwd, "getpwnam", return_value=account
            ), patch.object(MODULE.subprocess, "run", return_value=completed) as run, patch.dict(
                os.environ,
                {"PROVISION_USER": "tester", "WRDP_SOURCE_DIR": str(source)},
                clear=True,
            ):
                self.assertEqual(MODULE.main(), 0)
            argv = run.call_args.args[0]
            environment = run.call_args.kwargs["env"]
            self.assertEqual(argv[0], "/usr/sbin/runuser")
            self.assertEqual(argv[1:4], ["-u", "tester", "--"])
            self.assertEqual(argv[4], "/usr/bin/python3")
            self.assertEqual(environment["HOME"], "/home/tester")
            self.assertIn("/usr/sbin", environment["PATH"])


if __name__ == "__main__":
    unittest.main()
