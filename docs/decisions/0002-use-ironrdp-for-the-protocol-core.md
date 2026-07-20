# ADR 0002: Use IronRDP for the protocol core

**Status:** accepted
**Date:** 2026-07-19

## Decision

IronRDP owns RDP negotiation, CredSSP integration, capability exchange, protocol data units and dynamic-channel state machines. WRDP implements server traits and connects them to local desktop services.

## Why

RDP wire behavior is broad and stateful. Maintaining a second protocol core would add security and compatibility work without improving WRDP’s desktop integration.

## Consequences

- Wire-format changes should be made upstream when they are generally useful.
- WRDP keeps protocol adapters near their channel implementations.
- IronRDP revisions are pinned and tested with WRDP before release.
- Local policy must not be hidden inside protocol codecs.

## Alternatives considered

- **Maintain a separate RDP implementation:** rejected because protocol maintenance would dominate the project.
- **Fork protocol crates permanently:** rejected; temporary server hooks should remain suitable for upstreaming.
