# ADR 0004: Prefer EGFX with bitmap fallback

**Status:** accepted
**Date:** 2026-07-19

## Decision

Use the RDP Graphics Pipeline Extension with H.264 when negotiated. Support AVC420 and AVC444, optional VA-API encoding, and software H.264 fallback. Keep bitmap updates for clients or runtime paths that cannot use EGFX.

## Why

EGFX/H.264 reduces bandwidth and can use available encoders, but negotiation, drivers and hardware may fail. Bitmap updates provide a smaller compatibility floor and a recovery path.

## Consequences

- Encoder choice depends on negotiated capabilities and runtime availability.
- Hardware failure may replace the encoder with software H.264 during the session.
- A failed EGFX path can fall back to bitmap updates when the connection supports them.
- Damage and pacing decisions sit before the selected encoder.

## Alternatives considered

- **Require EGFX:** rejected because not every client and failure path can use it.
- **Bitmap only:** rejected because it wastes bandwidth and ignores available hardware encoding.
