# ADR 0009: Persist process identity for managed sessions

**Status:** accepted
**Date:** 2026-07-19

## Decision

The session manager records each component's PID, command, kernel start-time ticks, boot ID, UID, start timestamp and process-group identity. It checks that full identity before signalling a process.

Privileged lifecycle state and locks live under daemon-owned `/run/wrdp/sesman/<uid>`. User-owned Wayland sockets and component logs remain under `/run/user/<uid>/wrdp`. State files use mode `0600`, updates use a no-follow temporary file, `fsync` and atomic rename, and locks use a kernel advisory lock held by an open descriptor.

## Why

PIDs and process-group IDs are reused after process exit and across reboots. A stale state file must never cause WRDP to reuse or signal an unrelated process. The managed user cannot be allowed to replace the privileged record that authorizes teardown, even when the record describes that user's desktop.

## Consequences

- Session reuse requires matching user, session name, runtime directory, command, boot ID, UID, process start ticks and process group.
- Teardown signals a recorded process group only after all destructive-signal identity checks pass, then confirms exit after `SIGKILL` before removing state.
- Legacy or incomplete records may inform read-only status but fail closed for signalling.
- `wrdpctl` discovers only daemon-owned state beneath `/run/wrdp/sesman`; user-owned legacy registries are ignored.
- Startup reconciles interrupted client counts and idle deadlines; periodic scans perform only overdue idle cleanup.
- Partial startup and connection-binding failures run explicit rollback before ownership is released.
- Runtime and registry directories are temporary state and are rebuilt after reboot.

## Alternatives considered

- **Trust a persisted PID:** rejected because it may identify another process later.
- **Find sessions by process name:** rejected because names and command lines are not stable identities.
- **Store lifecycle state with the user's Wayland socket:** rejected because directory ownership would let the managed user replace process authority records and locks.
- **Store session state under `/tmp`:** rejected because ownership and cross-user cleanup are harder to enforce.
