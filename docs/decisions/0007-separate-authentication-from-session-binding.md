# ADR 0007: Separate authentication from session binding

**Status:** accepted
**Date:** 2026-07-19

## Decision

Credential validators prove identity and return accept or reject. They do not start a compositor or choose display and input handlers.

After authentication, IronRDP calls the connection binder. The binder normalizes the accepted username, asks session management to start or reuse that user's managed desktop, records the client attachment, and returns handlers bound to that session.

The current listener awaits one `run_connection()` call before accepting another. Validators therefore set the peer address immediately before the handshake and use it for rate limiting. Any future concurrent listener must move peer attribution into per-connection validator state before concurrency is enabled.

## Why

Starting a desktop during credential validation would allocate user resources before the protocol has completed authentication. Passing passwords into session management would also widen the secret-handling boundary without a need.

The post-auth binder is the first point where both identity and negotiated connection state are available.

## Consequences

- PAM and static-password validators do not create sessions.
- Session management receives a normalized username, not a password.
- Failed authentication cannot start a compositor.
- PAM work runs outside the async executor's worker thread.
- Rate limiting is keyed by peer address and depends on the serial-listener invariant.
- Listener concurrency requires a peer-context redesign and a separate review.

## Alternatives considered

- **Start sessions inside the credential validator:** rejected because validation may fail or the handshake may stop before binding.
- **Pass credentials into session management:** rejected because session lifecycle needs identity only.
- **Enable concurrent accepts with the current shared peer field:** rejected because attempts could be charged to the wrong address.
