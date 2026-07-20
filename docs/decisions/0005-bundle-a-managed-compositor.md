# ADR 0005: Bundle the managed compositor as a separate program

**Status:** accepted
**Date:** 2026-07-19

## Decision

WRDP ships the source for its labwc 0.8.3-derived compositor under `vendor/wrdp-compositor/`. Packaging builds it as a separate executable and installs it at `/usr/lib/wrdp/wrdp-compositor`.

The compositor remains GPL-2.0-only with its own license and notices. The Rust server remains MIT-licensed and communicates with the compositor through process, Wayland and frame-channel boundaries; it does not link compositor objects.

## Why

WRDP must create remote desktops without depending on, attaching to, or interfering with a user's GNOME or KDE session. Reusing the host desktop would overlap with its compositor, portals, display ownership, keybindings and login lifecycle, and would require a full desktop environment to be present.

A small headless compositor gives each authenticated user a separate desktop with the capture, input and clipboard interfaces WRDP expects. Keeping it as a separate program also preserves its upstream license boundary and keeps the Rust server independent of wlroots internals.

## Consequences

- The compositor tree includes its complete source, GPL license, original file notices, asset licenses and an upstream comparison record.
- WRDP-specific compositor changes are documented in the vendor subtree.
- Packaging must not combine compositor objects into the Rust binaries.
- The session manager starts the compositor under the authenticated user's identity.
- Updating the compositor requires a separate upstream comparison and license review.

## Alternatives considered

- **Require GNOME, KDE, or another system compositor:** rejected because WRDP would depend on and overlap with the user's local desktop session.
- **Link compositor code into WRDP:** rejected because it would couple the server to wlroots internals and blur the GPL/MIT boundary.
- **Maintain the compositor in a separate package only:** deferred until the runtime and packaging contract is stable enough to version independently.
