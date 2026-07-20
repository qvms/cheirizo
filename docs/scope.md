# Scope

## Supported direction

WRDP is an RDP server. It provides remote access to desktop sessions created and owned by WRDP on a Linux host.

## In scope

- Shared RDP listener with per-user sessions.
- TLS with PAM or configured-password validation.
- PAM and configured static-password validation.
- Managed Wayland compositor sessions.
- Direct frame capture from the managed compositor; PipeWire remains available for portal and audio integration.
- Keyboard, pointer, resize and clipboard channels.
- EGFX/H.264 with software and hardware backends.
- Bitmap fallback for clients or sessions that cannot use EGFX.

## Out of scope

- RDP client functionality.
- Domain-controller or directory-service implementation.
- NLA/CredSSP in the production single-daemon path.
- RDPSND audio redirection.
- Remote control of an arbitrary existing desktop session.
- Compatibility layers for removed internal APIs.
- Enterprise management or support features without a concrete use case.
