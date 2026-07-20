# ADR 0003: Own managed Wayland sessions

**Status:** accepted
**Date:** 2026-07-19

## Decision

WRDP starts and owns a Wayland compositor session for each authenticated user. Capture, input and clipboard attach to that managed session rather than an arbitrary existing desktop.

## Why

A server-owned session has clear credentials, lifecycle and resource ownership. Attaching to an existing desktop would depend on its compositor, portal state, logged-in user and local policy.

## Consequences

- The bundled compositor is part of the runtime and has its own license and notices.
- The managed compositor provides direct frames, input and clipboard; portal and PipeWire integrations remain separate runtime paths.
- Loss of the compositor ends attached RDP connections.
- Host desktop capture is not a fallback.

## Alternatives considered

- **Attach to the current graphical login:** rejected because ownership and authorization are ambiguous.
- **Support several compositor/session backends immediately:** rejected until there is another tested backend to justify the abstraction.
