# ADR 0001: Separate the server and session manager

**Status:** accepted
**Date:** 2026-07-19

## Decision

`wrdp` owns the listener, RDP connections and channel handlers. The session-manager library owns per-user compositor processes and persisted session state. The production server calls it in-process after authentication; `wrdp-sesman` is a CLI over the same manager, not a mandatory IPC service.

## Why

A shared listener must accept connections without inheriting one user’s desktop environment. Compositor processes also live longer than individual network connections and need a separate owner for reuse, cleanup and inspection.

## Consequences

- Connection failure does not implicitly destroy a reusable desktop session.
- Session-manager control is available through `wrdpctl`, not RDP channels.
- Credentials cross one post-auth binding boundary rather than being handled by display or channel code.
- Session health can terminate a connection without moving it to another backend.

## Alternatives considered

- **Start a compositor inside each connection task:** rejected because desktop lifecycle and network lifetime differ.
- **Run one shared desktop for every user:** rejected because users need isolated sessions and credentials.
