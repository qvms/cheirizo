# WRDP

WRDP is a single-port, multi-user RDP server for Linux servers running Wayland in headless mode.

It accepts an RDP connection, authenticates the user, starts or reuses that user's managed compositor session, applies the negotiated desktop size before capture, and sends display, input, audio and clipboard data through IronRDP.

## Goals

- One listener shared by multiple users.
- One isolated desktop session per authenticated user.
- Modern EGFX/H.264 display with a bitmap fallback.
- Software encoding plus optional VA-API acceleration.
- Clipboard, resize, audio and input support without exposing a host desktop session.
- A small operational surface: `wrdp`, `wrdp-sesman` and `wrdpctl`.

WRDP is built for Linux hosts where the server owns the desktop sessions. It is not a remote-control layer for an already logged-in graphical session, an RDP client, or a domain controller.

## Architecture

```text
RDP client
    │ TLS and RDP
    ▼
wrdp
    ├── credential validation
    ├── IronRDP protocol and channel handling
    ├── in-process session manager
    └── per-user session binding
             │
             ▼
       managed Wayland compositor
             ├── negotiated output size -> direct DMA-BUF/SHM capture
             ├── EGFX/H.264 or bitmap updates
             ├── ordered keyboard and pointer injection
             ├── PipeWire audio and clipboard
```

IronRDP owns the wire protocol, capability exchange and dynamic-channel state machines. WRDP connects those events to authentication, session management, Wayland, PipeWire and encoding backends. Advanced Input owns mouse delivery while its channel is active, which prevents the overlapping core input path from injecting the same click twice.

Read [`docs/architecture.md`](docs/architecture.md) for component boundaries, [`docs/compositor.md`](docs/compositor.md) for the managed desktop contract, and [`docs/decisions/`](docs/decisions/README.md) for design decisions.

## Installation and operation

The first public release is installed from source. The [operator guide](docs/operator-guide.md) covers supported deployment, build and installation, configuration, TLS/PAM security, systemd socket activation, packaging, upgrades, rollback and troubleshooting.

Distribution packages are not published yet; package maintainers should follow the documented packaging contract and keep the daemon, bundled compositor, lockfile and pinned IronRDP revision together.

## License

WRDP's Rust code is licensed under the MIT License. The bundled compositor is a modified labwc 0.8.3 derivative under GPL-2.0-only and is built as a separate program. IronRDP is Apache-2.0. See [`THIRD_PARTY.md`](THIRD_PARTY.md) for details.
