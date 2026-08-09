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
  -> bind authenticated user and negotiated desktop size to a session
  -> resize the managed output before capture
  -> construct display, input and dynamic-channel handlers transactionally
  -> stream until disconnect, timeout or supervised session failure
  -> stop supervision, close channels, join backend workers and release session ownership
```

Authentication and session creation stay separate: validating credentials must not start a compositor. The post-auth binder is the hand-off point.

## Desktop path

The session manager starts a managed Wayland compositor for each user. The server attaches through the desktop backend:

- The initial negotiated geometry passes the same allowlist, fixed-size and maximum-area policy as later resize requests. The managed compositor output is set and verified before capture begins, including when a healthy session is reused.
- The managed compositor supplies frames through a direct channel, using DMA-BUF when the renderer and driver permit it and SHM otherwise.
- PipeWire provides audio capture and remains available to portal-backed capture paths.
- Wayland or EIS interfaces inject keyboard and pointer input. Advanced Input owns mouse delivery while its DVC is active; IronRDP suppresses overlapping core mouse events for that connection. The ordered input queue is bounded; losing a release or synchronization event invalidates the connection instead of risking stuck state.
- data-control interfaces provide clipboard access. View-only sessions create neither virtual input nor clipboard backends and advertise neither path to RDP clients.

The server tears down an RDP connection when its managed desktop, capture or input path becomes permanently invalid. It does not fall back to another user's desktop or an unrelated host session. Resize is transactional: a requested mode is not published to RDP or input mapping until capture produces that exact geometry; newer requests supersede older ones.

## Session trust boundary

User runtime artifacts and privileged lifecycle authority are separate:

- `/run/user/<uid>/wrdp` is owned by the authenticated account and contains the Wayland socket and component logs.
- `/run/wrdp/sesman/<uid>` is owned by the daemon identity and contains the advisory lock and lifecycle state.

The daemon rejects symlinked or wrongly owned runtime components before privileged access. Compositor control runs under the authenticated UID/GID, not root. Persisted process authority includes the boot ID, UID, PID start ticks and process group; destructive signalling requires every field to match. Legacy user-owned registries are never trusted as signal authority.

Startup reconciles stale client counts and persisted idle deadlines. A daemon-wide periodic pass performs only overdue idle cleanup and never rewrites live client ownership.

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
| `src/rdp/channels/input/` | keyboard, pointer, coordinate transforms and display-control input |
| `src/rdp/channels/clipboard/` | CLIPRDR formats, ownership, transfer and policy |
| `src/desktop/` | Wayland, PipeWire, compositor and portal integration |
| `src/auth/` | credential validation, username normalization and throttling |
| `src/security/` | TLS and certificate handling |
| `src/sesman/` | per-user compositor process lifecycle |
| `src/services/` | host capability and service discovery |

Protocol/channel code stays with the channel that owns it. Generic `utils`, `common` or `helpers` modules are avoided when a domain owner exists.
