#!/usr/bin/env python3
"""Measure WRDP cgroup and session process memory without changing system state."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import statistics
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

_MAX_PROC_FILE = 4 * 1024 * 1024
_RUNTIME_RE = re.compile(rb"XDG_RUNTIME_DIR=/run/user/([0-9]+)/wrdp\Z")
_SMAP_KEYS = (
    "Rss",
    "Pss",
    "Private_Clean",
    "Private_Dirty",
    "Shared_Clean",
    "Shared_Dirty",
)
_SOURCE_ORDER = {"cgroup": 0, "session_environment": 1, "state_component": 2, "state_descendant": 3}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def error_kind(exc: OSError) -> str:
    if isinstance(exc, FileNotFoundError):
        return "not_found"
    if isinstance(exc, PermissionError):
        return "permission_denied"
    return "os_error"


def read_bytes(path: Path, limit: int = _MAX_PROC_FILE) -> tuple[bytes | None, str | None]:
    try:
        with path.open("rb") as handle:
            data = handle.read(limit + 1)
    except OSError as exc:
        return None, error_kind(exc)
    if len(data) > limit:
        return None, "too_large"
    return data, None


def read_text(path: Path, limit: int = _MAX_PROC_FILE) -> tuple[str | None, str | None]:
    data, error = read_bytes(path, limit)
    if data is None:
        return None, error
    return data.decode("utf-8", "replace"), None


def parse_nonnegative_int(value: str) -> int:
    parsed = int(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be non-negative")
    return parsed


def parse_positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def service_name(value: str) -> str:
    if not value or value in {".", ".."} or "/" in value or "\x00" in value:
        raise argparse.ArgumentTypeError("must be a single cgroup name")
    return value


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", required=True, help="scenario label")
    parser.add_argument("--samples", type=parse_positive_int, default=5)
    parser.add_argument("--interval", type=float, default=2.0, help="seconds between samples")
    parser.add_argument("--output", type=Path, help="write JSON to this file instead of stdout")
    parser.add_argument(
        "--evidence-json",
        type=Path,
        help="embed a bounded, non-secret acceptance evidence object",
    )
    parser.add_argument("--service", type=service_name, default="wrdp.service")
    parser.add_argument("--session-uid", type=parse_nonnegative_int)
    parser.add_argument("--proc-root", type=Path, default=Path("/proc"), help=argparse.SUPPRESS)
    parser.add_argument(
        "--cgroup-root", type=Path, default=Path("/sys/fs/cgroup"), help=argparse.SUPPRESS
    )
    parser.add_argument(
        "--registry-root", type=Path, default=Path("/run/wrdp/sesman"), help=argparse.SUPPRESS
    )
    args = parser.parse_args(argv)
    if not math.isfinite(args.interval) or args.interval < 0:
        parser.error("--interval must be a finite non-negative number")
    return args


def load_evidence(path: Path | None) -> dict[str, Any] | None:
    if path is None:
        return None
    text, error = read_text(path, 64 * 1024)
    if text is None:
        raise ValueError(f"cannot read evidence: {error}")
    try:
        evidence = json.loads(text)
    except (json.JSONDecodeError, RecursionError) as exc:
        raise ValueError("evidence is not valid JSON") from exc
    if not isinstance(evidence, dict):
        raise ValueError("evidence must be a JSON object")
    return evidence


def json_int(value: Any) -> int | None:
    return value if type(value) is int and value >= 0 else None


def requested_size(value: Any) -> dict[str, int] | None:
    if not isinstance(value, dict):
        return None
    width = json_int(value.get("width"))
    height = json_int(value.get("height"))
    if width is None or height is None:
        return None
    return {"width": width, "height": height}


def load_state(uid: int, registry_root: Path) -> dict[str, Any]:
    result: dict[str, Any] = {
        "session_uid": uid,
        "requested_size": None,
        "active_clients": None,
        "state_available": False,
        "component_identities": [],
    }
    data, error = read_text(registry_root / str(uid) / "default.state.json")
    if data is None:
        result["state_error"] = error
        return result
    try:
        state = json.loads(data)
    except (json.JSONDecodeError, RecursionError):
        result["state_error"] = "invalid_json"
        return result
    if not isinstance(state, dict):
        result["state_error"] = "invalid_json"
        return result

    result["state_available"] = True
    result["requested_size"] = requested_size(state.get("requested_size"))
    result["active_clients"] = json_int(state.get("active_clients"))
    components = state.get("components")
    if isinstance(components, list):
        identities = []
        for component in components:
            if not isinstance(component, dict):
                continue
            pid = component.get("pid")
            start_ticks = component.get("start_ticks")
            boot_id = component.get("boot_id")
            component_uid = component.get("uid")
            if (
                type(pid) is int
                and pid > 0
                and type(start_ticks) is int
                and start_ticks >= 0
                and isinstance(boot_id, str)
                and boot_id
                and type(component_uid) is int
                and component_uid == uid
            ):
                identities.append(
                    {
                        "pid": pid,
                        "start_ticks": start_ticks,
                        "boot_id": boot_id,
                        "uid": component_uid,
                    }
                )
        result["component_identities"] = sorted(identities, key=lambda item: item["pid"])
    return result


def state_uids(registry_root: Path, selected_uid: int | None) -> tuple[list[int], str | None]:
    if selected_uid is not None:
        return [selected_uid], None
    try:
        entries = list(registry_root.iterdir())
    except OSError as exc:
        return [], error_kind(exc)
    uids = sorted(
        int(entry.name)
        for entry in entries
        if re.fullmatch(r"[0-9]+", entry.name)
        and (entry / "default.state.json").is_file()
    )
    return uids, None


def load_sessions(registry_root: Path, selected_uid: int | None) -> tuple[list[dict[str, Any]], str | None]:
    uids, error = state_uids(registry_root, selected_uid)
    return [load_state(uid, registry_root) for uid in uids], error


def public_scenario(sessions: list[dict[str, Any]], registry_error: str | None) -> dict[str, Any]:
    public = []
    for session in sorted(sessions, key=lambda item: item["session_uid"]):
        item = {key: value for key, value in session.items() if key != "component_identities"}
        public.append(item)
    result: dict[str, Any] = {"sessions": public}
    if len(public) == 1:
        result.update(public[0])
    else:
        result.update({"session_uid": None, "requested_size": None, "active_clients": None})
    if registry_error is not None:
        result["registry_error"] = registry_error
    return result


def list_proc_pids(proc_root: Path) -> tuple[list[int], str | None]:
    try:
        entries = list(proc_root.iterdir())
    except OSError as exc:
        return [], error_kind(exc)
    return sorted(
        int(entry.name)
        for entry in entries
        if re.fullmatch(r"[0-9]+", entry.name) and entry.is_dir()
    ), None


def parse_status(text: str) -> dict[str, int | str | None]:
    fields: dict[str, str] = {}
    for line in text.splitlines():
        key, separator, value = line.partition(":")
        if separator:
            fields[key] = value.strip()

    def first_int(key: str) -> int | None:
        value = fields.get(key, "").split()
        if not value:
            return None
        try:
            parsed = int(value[0])
        except ValueError:
            return None
        return parsed if parsed >= 0 else None

    return {
        "name": fields.get("Name"),
        "uid": first_int("Uid"),
        "ppid": first_int("PPid"),
        "vmrss_kib": first_int("VmRSS"),
    }


def read_status(proc_root: Path, pid: int) -> tuple[dict[str, int | str | None], str | None]:
    text, error = read_text(proc_root / str(pid) / "status")
    if text is None:
        return {"name": None, "uid": None, "ppid": None, "vmrss_kib": None}, error
    return parse_status(text), None


def cgroup_snapshot(cgroup_root: Path, service: str) -> tuple[dict[str, Any], list[int], list[str]]:
    directory = cgroup_root / "system.slice" / service
    errors: list[str] = []

    def memory_value(name: str) -> int | None:
        text, error = read_text(directory / name, 256)
        if text is None:
            errors.append(f"cgroup.{name}: {error}")
            return None
        try:
            value = int(text.strip())
        except ValueError:
            errors.append(f"cgroup.{name}: invalid_integer")
            return None
        if value < 0:
            errors.append(f"cgroup.{name}: invalid_integer")
            return None
        return value

    current = memory_value("memory.current")
    peak = memory_value("memory.peak")
    procs_text, error = read_text(directory / "cgroup.procs")
    pids: list[int] = []
    if procs_text is None:
        errors.append(f"cgroup.cgroup.procs: {error}")
    else:
        invalid = False
        for token in procs_text.split():
            try:
                pid = int(token)
            except ValueError:
                invalid = True
                continue
            if pid > 0:
                pids.append(pid)
            else:
                invalid = True
        if invalid:
            errors.append("cgroup.cgroup.procs: invalid_pid")
    pids = sorted(set(pids))
    return {
        "memory_current_bytes": current,
        "memory_peak_bytes": peak,
        "pids": pids,
    }, pids, errors


def process_start_ticks(proc_root: Path, pid: int) -> int | None:
    text, _ = read_text(proc_root / str(pid) / "stat", 64 * 1024)
    if text is None or ") " not in text:
        return None
    try:
        value = int(text.rsplit(") ", 1)[1].split()[19])
    except (IndexError, ValueError):
        return None
    return value if value >= 0 else None


def boot_id(proc_root: Path) -> str | None:
    text, _ = read_text(proc_root / "sys/kernel/random/boot_id", 4096)
    if text is None:
        return None
    value = text.strip()
    return value or None


def environment_uid(proc_root: Path, pid: int, selected_uid: int | None) -> int | None:
    data, _ = read_bytes(proc_root / str(pid) / "environ")
    if data is None:
        return None
    for entry in data.split(b"\0"):
        match = _RUNTIME_RE.fullmatch(entry)
        if match is None:
            continue
        uid = int(match.group(1))
        if selected_uid is None or uid == selected_uid:
            return uid
    return None


def redact_cmdline(data: bytes) -> dict[str, Any]:
    argv = [part for part in data.split(b"\0") if part]
    if not argv:
        return {"executable": None, "argument_count": 0, "arguments_redacted": True}
    executable = argv[0].decode("utf-8", "replace").rsplit("/", 1)[-1]
    return {
        "executable": executable,
        "argument_count": max(0, len(argv) - 1),
        "arguments_redacted": True,
    }


def read_smaps(proc_root: Path, pid: int) -> tuple[dict[str, int | None], str | None]:
    values = {f"{key.lower()}_kib": None for key in _SMAP_KEYS}
    text, error = read_text(proc_root / str(pid) / "smaps_rollup")
    if text is None:
        return values, error
    wanted = set(_SMAP_KEYS)
    for line in text.splitlines():
        key, separator, rest = line.partition(":")
        if not separator or key not in wanted:
            continue
        parts = rest.split()
        if not parts:
            continue
        try:
            value = int(parts[0])
        except ValueError:
            continue
        if value >= 0:
            values[f"{key.lower()}_kib"] = value
    return values, None


def process_snapshot(
    proc_root: Path,
    pid: int,
    sources: set[str],
    session_uids: set[int],
    cached_status: tuple[dict[str, int | str | None], str | None] | None,
) -> dict[str, Any]:
    errors: list[str] = []
    status, status_error = cached_status or read_status(proc_root, pid)
    if status_error is not None:
        errors.append(f"status: {status_error}")

    comm_text, comm_error = read_text(proc_root / str(pid) / "comm", 4096)
    if comm_text is None:
        comm = status.get("name")
        errors.append(f"comm: {comm_error}")
    else:
        comm = comm_text.rstrip("\r\n")

    cmdline_data, cmdline_error = read_bytes(proc_root / str(pid) / "cmdline")
    if cmdline_data is None:
        cmdline = None
        errors.append(f"cmdline: {cmdline_error}")
    else:
        cmdline = redact_cmdline(cmdline_data)

    smaps, smaps_error = read_smaps(proc_root, pid)
    if smaps_error is not None:
        errors.append(f"smaps_rollup: {smaps_error}")

    result: dict[str, Any] = {
        "pid": pid,
        "sources": sorted(sources, key=lambda source: _SOURCE_ORDER[source]),
        "session_uids": sorted(session_uids),
        "comm": comm,
        "cmdline": cmdline,
        "uid": status.get("uid"),
        "ppid": status.get("ppid"),
        "vmrss_kib": status.get("vmrss_kib"),
        "smaps_rollup": smaps,
    }
    if errors:
        result["errors"] = errors
    return result


def associated_processes(
    proc_root: Path,
    cgroup_pids: Iterable[int],
    sessions: list[dict[str, Any]],
    selected_uid: int | None,
) -> tuple[list[dict[str, Any]], set[int], list[str]]:
    proc_pids, proc_error = list_proc_pids(proc_root)
    errors = [] if proc_error is None else [f"proc: {proc_error}"]
    status_cache = {pid: read_status(proc_root, pid) for pid in proc_pids}
    parent = {
        pid: status[0].get("ppid")
        for pid, status in status_cache.items()
        if type(status[0].get("ppid")) is int
    }

    sources: dict[int, set[str]] = {}
    process_uids: dict[int, set[int]] = {}
    for pid in cgroup_pids:
        status = status_cache.get(pid)
        uid = status[0].get("uid") if status is not None else None
        if selected_uid is not None and uid not in {0, selected_uid}:
            continue
        sources.setdefault(pid, set()).add("cgroup")
        process_uids.setdefault(pid, set())

    discovered_uids: set[int] = set()
    for pid in proc_pids:
        uid = environment_uid(proc_root, pid, selected_uid)
        if uid is not None:
            discovered_uids.add(uid)
            sources.setdefault(pid, set()).add("session_environment")
            process_uids.setdefault(pid, set()).add(uid)

    root_uids: dict[int, set[int]] = {}
    current_boot_id = boot_id(proc_root)
    for session in sessions:
        uid = session["session_uid"]
        for identity in session["component_identities"]:
            pid = identity["pid"]
            status = status_cache.get(pid)
            live_uid = status[0].get("uid") if status is not None else None
            if (
                live_uid == identity["uid"]
                and current_boot_id == identity["boot_id"]
                and process_start_ticks(proc_root, pid) == identity["start_ticks"]
            ):
                root_uids.setdefault(pid, set()).add(uid)

    for pid in proc_pids:
        current = pid
        visited: set[int] = set()
        while current > 0 and current not in visited:
            visited.add(current)
            if current in root_uids:
                relation = "state_component" if pid == current else "state_descendant"
                sources.setdefault(pid, set()).add(relation)
                process_uids.setdefault(pid, set()).update(root_uids[current])
                discovered_uids.update(root_uids[current])
                break
            next_pid = parent.get(current)
            if type(next_pid) is not int:
                break
            current = next_pid

    processes = [
        process_snapshot(
            proc_root,
            pid,
            pid_sources,
            process_uids.get(pid, set()),
            status_cache.get(pid),
        )
        for pid, pid_sources in sorted(sources.items())
    ]
    return processes, discovered_uids, errors


def process_rss(process: dict[str, Any]) -> int | None:
    smaps_rss = process["smaps_rollup"]["rss_kib"]
    return smaps_rss if smaps_rss is not None else process["vmrss_kib"]


def sum_known(values: list[int | None], empty_value: int = 0) -> int | None:
    if not values:
        return empty_value
    if any(value is None for value in values):
        return None
    return sum(value for value in values if value is not None)


def process_totals(processes: list[dict[str, Any]]) -> dict[str, Any]:
    rss_values = [process_rss(process) for process in processes]
    pss_values = [process["smaps_rollup"]["pss_kib"] for process in processes]
    return {
        "unique_process_count": len(processes),
        "rss_kib": sum_known(rss_values),
        "pss_kib": sum_known(pss_values),
        "rss_processes_measured": sum(value is not None for value in rss_values),
        "pss_processes_measured": sum(value is not None for value in pss_values),
        "rss_complete": all(value is not None for value in rss_values),
        "pss_complete": all(value is not None for value in pss_values),
    }


def merge_discovered_sessions(
    sessions: list[dict[str, Any]], discovered_uids: set[int], registry_root: Path
) -> list[dict[str, Any]]:
    existing = {session["session_uid"] for session in sessions}
    return sessions + [load_state(uid, registry_root) for uid in sorted(discovered_uids - existing)]


def collect_sample(args: argparse.Namespace) -> dict[str, Any]:
    cgroup, cgroup_pids, errors = cgroup_snapshot(args.cgroup_root, args.service)
    sessions, registry_error = load_sessions(args.registry_root, args.session_uid)
    processes, discovered_uids, proc_errors = associated_processes(
        args.proc_root, cgroup_pids, sessions, args.session_uid
    )
    sessions = merge_discovered_sessions(sessions, discovered_uids, args.registry_root)
    result: dict[str, Any] = {
        "timestamp": utc_now(),
        "cgroup": cgroup,
        "scenario": public_scenario(sessions, registry_error),
        "processes": processes,
        "process_totals": process_totals(processes),
    }
    errors.extend(proc_errors)
    if errors:
        result["errors"] = errors
    return result


def stats(values: Iterable[int | float | None]) -> dict[str, int | float | None]:
    known = [value for value in values if value is not None]
    if not known:
        return {"mean": None, "min": None, "max": None, "stdev": None}
    return {
        "mean": statistics.fmean(known),
        "min": min(known),
        "max": max(known),
        "stdev": statistics.pstdev(known),
    }


def per_comm_summary(samples: list[dict[str, Any]]) -> dict[str, dict[str, float | int | None]]:
    comms = sorted(
        {str(process["comm"] or "<unknown>") for sample in samples for process in sample["processes"]}
    )
    result: dict[str, dict[str, float | int | None]] = {}
    for comm in comms:
        counts: list[int] = []
        rss_totals: list[int | None] = []
        pss_totals: list[int | None] = []
        for sample in samples:
            matching = [
                process
                for process in sample["processes"]
                if str(process["comm"] or "<unknown>") == comm
            ]
            counts.append(len(matching))
            rss_totals.append(sum_known([process_rss(process) for process in matching]))
            pss_totals.append(
                sum_known([process["smaps_rollup"]["pss_kib"] for process in matching])
            )
        result[comm] = {
            "mean_rss_kib": stats(rss_totals)["mean"],
            "mean_pss_kib": stats(pss_totals)["mean"],
            "mean_process_count": stats(counts)["mean"],
        }
    return result


def summarize(samples: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "cgroup": {
            "memory_current_bytes": stats(
                sample["cgroup"]["memory_current_bytes"] for sample in samples
            ),
            "memory_peak_bytes": stats(
                sample["cgroup"]["memory_peak_bytes"] for sample in samples
            ),
        },
        "total_unique_process_memory": {
            "rss_kib": stats(sample["process_totals"]["rss_kib"] for sample in samples),
            "pss_kib": stats(sample["process_totals"]["pss_kib"] for sample in samples),
        },
        "per_comm": per_comm_summary(samples),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    started_at = utc_now()
    evidence = load_evidence(args.evidence_json)
    samples = []
    for index in range(args.samples):
        samples.append(collect_sample(args))
        if index + 1 < args.samples:
            time.sleep(args.interval)
    report = {
        "schema_version": 1,
        "label": args.label,
        "started_at": started_at,
        "completed_at": utc_now(),
        "configuration": {
            "samples": args.samples,
            "interval_seconds": args.interval,
            "service": args.service,
            "session_uid": args.session_uid,
        },
        "scenario": samples[0]["scenario"] if samples else public_scenario([], None),
        "samples": samples,
        "summary": summarize(samples),
    }
    if evidence is not None:
        report["evidence"] = evidence
    return report


def emit(report: dict[str, Any], output: Path | None) -> None:
    rendered = json.dumps(report, indent=2, sort_keys=True, ensure_ascii=True) + "\n"
    if output is None:
        sys.stdout.write(rendered)
        return
    with output.open("w", encoding="utf-8") as handle:
        handle.write(rendered)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        report = run(args)
        emit(report, args.output)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 2
    except OSError as exc:
        print(f"cannot write output: {error_kind(exc)}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
