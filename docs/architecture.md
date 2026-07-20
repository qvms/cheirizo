# Architecture

WRDP separates the network-facing RDP server from per-user desktop lifecycle management.

## Processes

### `wrdp`

The server owns the TCP listener, TLS, credential validation, the IronRDP frontend, negotiated channels, and the connection-to-session binding. It does not own compositor processes.

### Session manager

The production server calls the session-manager library in-process to start or reuse per-user compositor processes and persist their runtime state. `wrdp-sesman` exposes the same manager as a command-line tool for foreground supervision and manual session operations; it is not a required IPC daemon.

### `wrdpctl`

The administrative client inspects and controls the persisted session-manager state without exposing that control surface through RDP.

## Connection lifecycle

```text
TCP accept
  -> TLS and RDP negotiation
  -> PAM or configured-password validation
  -> bind authenticated user to a session
  -> construct display, input and dynamic-channel handlers
  -> stream until disconnect or session failure
  -> close channels and release connection-owned resources
```

Authentication and session creation stay separate: validating credentials must not start a compositor. The post-auth binder is the hand-off point.

## Desktop path

The session manager starts a managed Wayland compositor for each user. The server attaches through the desktop backend:

- The managed compositor supplies frames through a direct channel.
- PipeWire remains available for portal capture paths and audio integration.
- Wayland or EIS interfaces inject keyboard and pointer input.
- data-control interfaces provide clipboard access.

The server tears down an RDP connection when its managed desktop disappears. It does not fall back to another user’s desktop or an unrelated host session.

## Display path

```text
compositor frame
  -> frame and pixel-format normalization
  -> damage and pacing decisions
  -> negotiated encoder
       -> EGFX AVC420/AVC444
       -> VA-API when enabled and available
       -> software H.264 fallback
       -> bitmap fallback when EGFX is unavailable
  -> IronRDP display/channel output
```

Negotiated client capabilities select the path. Configuration may disable an encoder, but it cannot create a protocol capability the client did not negotiate.

## Source ownership

| Path | Owner |
|---|---|
| `src/rdp/server/` | listener, IronRDP frontend, connection lifecycle and post-auth binding |
| `src/rdp/session/` | desktop-session handles, backend selection and supervision |
| `src/rdp/channels/graphics/` | display updates, EGFX, H.264, damage and pacing |
| `src/rdp/channels/input/` | keyboard, pointer and display-control input |
| `src/rdp/channels/clipboard/` | CLIPRDR formats, ownership, transfer and policy |
| `src/desktop/` | Wayland, PipeWire, compositor and portal integration |
| `src/auth/` | credential validation, username normalization and throttling |
| `src/security/` | TLS and certificate handling |
| `src/sesman/` | per-user compositor process lifecycle |
| `src/services/` | host capability and service discovery |

Protocol/channel code stays with the channel that owns it. Generic `utils`, `common` or `helpers` modules are avoided when a domain owner exists.
