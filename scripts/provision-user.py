#!/usr/bin/env python3
"""Run WRDP's user ground-init profile under one validated local account."""

from __future__ import annotations

import os
import pwd
import re
import subprocess
from pathlib import Path


VALID_USER = re.compile(r"[a-z_][a-z0-9_.-]{0,31}\Z")


def main() -> int:
    if os.geteuid() != 0:
        raise SystemExit("provision-user must run as root")
    username = os.environ.get("PROVISION_USER", "")
    if not VALID_USER.fullmatch(username):
        raise SystemExit("PROVISION_USER must be a simple local account name")
    try:
        account = pwd.getpwnam(username)
    except KeyError as error:
        raise SystemExit(f"unknown local account: {username}") from error
    home = Path(account.pw_dir)
    if not home.is_absolute() or home == Path("/"):
        raise SystemExit(f"unsafe home directory for {username}: {home}")
    source = Path(os.environ.get("WRDP_SOURCE_DIR", "")).resolve()
    runner = source / "vendor/ground-init/ground-init.py"
    profile = source / "ground-init.user.yaml"
    if not runner.is_file() or not profile.is_file():
        raise SystemExit("WRDP_SOURCE_DIR does not contain the user profile")

    environment = {
        "HOME": str(home),
        "LANG": os.environ.get("LANG", "C.UTF-8"),
        "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    }
    completed = subprocess.run(
        [
            "/usr/sbin/runuser",
            "-u",
            username,
            "--",
            "/usr/bin/python3",
            str(runner),
            str(profile),
        ],
        env=environment,
        check=False,
    )
    return completed.returncode


if __name__ == "__main__":
    raise SystemExit(main())
