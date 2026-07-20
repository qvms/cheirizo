# ADR 0009: Persist process identity for managed sessions

**Status:** accepted
**Date:** 2026-07-19

## Decision

The session manager records each component's PID, command, kernel start-time ticks, start timestamp and process-group ownership in a per-user state file. It checks that identity before reusing or signalling a process.

State, locks and logs live under `/run/user/<uid>/wrdp`, owned by the authenticated user with mode `0700`; state and lock files use mode `0600`. State updates use a temporary file, `fsync` and atomic rename.

## Why

PIDs are reused after process exit and across reboots. A stale state file must never cause WRDP to reuse or signal an unrelated process. Per-user ownership also prevents one session from replacing another user's registry or logs.

## Consequences

- Session reuse requires matching user, session name, runtime directory, command and process start ticks.
- Teardown signals the recorded process group only after identity checks pass.
- Older PID-only records use a timestamp fallback and are replaced on the next successful start.
- `wrdpctl` ignores state files whose owner does not match the `/run/user/<uid>` tree and resolved account.
- The session manager removes partial process trees after startup failure.
- Runtime directories are temporary state and are rebuilt after reboot.

## Alternatives considered

- **Trust a persisted PID:** rejected because it may identify another process later.
- **Find sessions by process name:** rejected because names and command lines are not stable identities.
- **Store session state under `/tmp`:** rejected because ownership and cross-user cleanup are harder to enforce.
