# ADR 0010: Use one RDP server lifecycle

**Status:** accepted
**Date:** 2026-07-20

## Decision

WRDP has one listener and connection lifecycle, owned by `src/rdp/server/`. The `wrdp` binary parses configuration and diagnostic commands, then calls that server entrypoint.

Authentication, post-auth session binding, display/input handlers, clipboard, EGFX, socket activation and disconnect cleanup are wired once. Alternative desktop backends remain behind the session backend interface; they do not create a second listener implementation.

## Why

The implementation contained two complete server paths: the normally executed single-daemon path in `src/bin/wrdp.rs` and an older `WrdpRdpServer` orchestrator. The binary always selected the former, leaving the latter unreachable while both continued to evolve. They disagreed on authentication modes, capture, audio, CLI overrides and cleanup.

One lifecycle makes supported behavior visible and keeps channel policy out of the CLI.

## Consequences

- `src/bin/wrdp.rs` owns CLI and diagnostics only.
- `src/rdp/server/` owns listener startup through disconnect cleanup.
- CLI listener overrides are applied before server startup.
- New channel or authentication features are wired in one place.
- When Advanced Input is active, it owns mouse delivery and the overlapping core mouse path is suppressed for that connection.
- Removed server paths do not receive compatibility shims in the new public history.

## Alternatives considered

- **Keep both paths:** rejected because one was unreachable and already contradicted production behavior.
- **Select a path by executable name or environment variable:** rejected because it makes the supported server ambiguous.
- **Move all server code into the binary:** rejected because connection lifecycle and binders belong to the library's RDP server domain.
