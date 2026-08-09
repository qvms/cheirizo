# WRDP operator guide

This guide describes the first public source release. WRDP currently ships source and a bundled compositor; distribution packages are not yet published. The optional [minimal Platinum desktop](minimal-desktop.md) documents the Waybar, Thunar, Mako and two-pass ground-init profile.

## Supported deployment

WRDP targets a dedicated Linux host running systemd and Wayland dependencies. The daemon owns one managed compositor session per authenticated local user. It does not attach to an existing desktop. Two build compositions are supported:

- Cargo defaults: PAM authentication, OpenH264, VA-API, Wayland, RDPSND and the direct portal backend.
- Portable software: `--no-default-features --features h264,wayland,portal-generic`, using configured Argon2id credentials and software H.264.

A bare `--no-default-features` build is not supported because the only production session backend requires Wayland and the graphics path requires an H.264 implementation.

## Build and install

On Debian or Ubuntu:

```sh
git clone https://github.com/rcarmo/wrdp.git
cd wrdp
make bootstrap
make ci
sudo install -m 0755 .local/tmp/wrdp-target/debug/{wrdp,wrdp-sesman,wrdpctl} /usr/local/bin/
sudo install -d -m 0755 /usr/lib/wrdp /etc/wrdp
sudo install -m 0755 .local/tmp/wrdp-compositor-build/wrdp-compositor /usr/lib/wrdp/
sudo make install-session-defaults
```

For a complete minimal desktop, including packages and per-user preferences, use the two provisioning passes instead:

```sh
sudo make provision-system
sudo make provision-user PROVISION_USER="$USER"
```

Use `make build-release` and the corresponding `release/` paths for an optimized deployment. Keep the daemon and bundled compositor from the same source revision.

## Initial configuration

Generate the authoritative template rather than copying a stale example:

```sh
sudo sh -c 'wrdp --generate-config > /etc/wrdp/wrdp.ini'
sudo chmod 0640 /etc/wrdp/wrdp.ini
sudo chown root:root /etc/wrdp/wrdp.ini
```

The production daemon reads `/etc/wrdp/wrdp.ini` by default. `--config PATH`, `--listen`, and `--port` override it. Environment overrides use nested names such as `WRDP_SERVER__LISTEN_ADDR`.

At minimum, review:

- `server.listen_addr`; do not expose TCP 3389 beyond intended networks.
- `server.max_connections`; production currently requires exactly `1`. The serial admission path applies a 30-second pre-authentication deadline.
- `server.session_timeout`; `0` leaves authenticated sessions unlimited, otherwise the value is the maximum session lifetime in seconds.
- `server.view_only`; when enabled, WRDP creates no virtual input or clipboard backend and exposes neither input nor CLIPRDR to the client.
- `security.cert_path` and `security.key_path`.
- `security.auth_method` (`pam` or `password`).
- `security.allowed_username`, if the host should accept only one account.
- clipboard type, size and rate limits.
- display resize and resolution limits. Initial negotiation and later Display Control requests use the same policy. WRDP verifies the realized compositor mode and matching capture frame before publishing a size.
- EGFX software/hardware encoding policy and `hardware_encoding.vaapi_device`.
- `video.cursor_mode`; managed sessions normally hide the captured compositor cursor and let the RDP client render its local pointer.

Validate before restart:

```sh
sudo wrdp --config /etc/wrdp/wrdp.ini --diagnose
```

## TLS and authentication

WRDP always uses TLS in the production lifecycle. Generate a temporary self-signed identity only for controlled testing:

```sh
sudo openssl req -x509 -newkey rsa:4096 -nodes \
  -keyout /etc/wrdp/key.pem -out /etc/wrdp/cert.pem \
  -days 365 -subj '/CN=wrdp.example.net'
sudo chmod 0600 /etc/wrdp/key.pem
sudo chmod 0644 /etc/wrdp/cert.pem
```

Use a CA-issued certificate in production and distribute the issuing trust chain to clients. Never copy the private key into logs, issue reports or package artifacts.

PAM is the normal mode and validates the RDP credentials against a local account. Optionally restrict it with `allowed_username`. Static-password mode requires an Argon2id PHC hash for every configured RDP username; plaintext passwords are rejected. NLA/CredSSP and domain authentication are not part of this release.

Run the daemon with the privileges needed to bind the listener, validate PAM users, create trusted lifecycle state and drop managed components to their target UID/GID. Do not run the compositor as root.

## Runtime and lifecycle state

WRDP deliberately separates user-controlled runtime files from privileged lifecycle authority:

- `/run/user/UID/wrdp` is owned by the managed account and contains the Wayland socket and component logs.
- `/run/wrdp/sesman/UID` is owned by the daemon identity and contains the session state and kernel advisory lock.

Do not change ownership of `/run/wrdp/sesman` or copy state files into it from user-owned paths. WRDP ignores legacy `/run/user/UID/wrdp/sesman` registries and never uses them to signal processes. On first start after this change, existing managed compositor processes from an older build may need to be stopped manually before reconnecting.

At daemon startup, trusted registries are reconciled after an interrupted connection. Idle deadlines are persisted, and a periodic scanner stops overdue idle sessions without altering live client counts.

## systemd socket activation

WRDP accepts systemd socket descriptor 0. A minimal deployment uses:

```ini
# /etc/systemd/system/wrdp.socket
[Unit]
Description=WRDP listener

[Socket]
ListenStream=3389
Accept=no
NoDelay=true

[Install]
WantedBy=sockets.target
```

```ini
# /etc/systemd/system/wrdp.service
[Unit]
Description=WRDP Wayland RDP daemon
Requires=wrdp.socket
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/wrdp --config /etc/wrdp/wrdp.ini
Restart=on-failure
RestartSec=2s
RuntimeDirectory=wrdp
RuntimeDirectoryMode=0700
RuntimeDirectoryPreserve=yes
ReadWritePaths=/run/wrdp /run/user
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

Then run:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now wrdp.socket
sudo systemctl status wrdp.socket wrdp.service
```

Do not set both a systemd socket and an unrelated process to the same address. WRDP requires the activated descriptor to be a listening TCP socket whose complete address and IP family exactly match `server.listen_addr`; mismatch is a startup error. Keep `RuntimeDirectoryPreserve=yes`: trusted session state must survive a daemon restart so startup reconciliation can authenticate or retire managed processes safely.

## Packaging contract

A package should install:

- `wrdp`, `wrdp-sesman`, and `wrdpctl` in `/usr/bin` or `/usr/local/bin`.
- the GPL-2.0-only bundled compositor as a separate executable in `/usr/lib/wrdp`.
- the Platinum theme in `/usr/share/themes/PlatinumTheme-wrdp-compositor` plus Labwc and Waybar defaults under `/etc/wrdp`.
- configuration in `/etc/wrdp`, preserving local edits on upgrade.
- systemd units in the distribution unit directory.
- `LICENSE`, `THIRD_PARTY.md`, and the compositor GPL license.

Build from `Cargo.lock` with `--locked`, Rust 1.94 or newer, and the pinned IronRDP Git revision. Packages must not silently substitute another compositor or IronRDP revision.

## Upgrade and rollback

Before upgrading:

1. Record `wrdp --version` and the package/source revision.
2. Back up `/etc/wrdp`, including certificate metadata but protect private keys.
3. Stop `wrdp.socket` to prevent new connections, then stop `wrdp.service`.
4. Stop existing managed sessions with `wrdpctl`; older user-owned state is intentionally not imported into the trusted registry.
5. Install the daemon and bundled compositor atomically from the same release.
6. Verify `/run/wrdp` is root-owned and not writable by managed users.
7. Run `wrdp --diagnose`, then start the socket and test a new session, resize, disconnect and reconnect.

Configuration additions have defaults, but `server.max_connections` must be `1`; review any previous higher value before restart. For rollback, stop the socket and service, stop managed sessions, remove only the temporary root-owned `/run/wrdp/sesman` registry, restore the previous binaries, matching compositor and configuration, run diagnostics, then restart the socket. Never move new root-owned state into an older user-owned registry or vice versa. Session state is operational state, not a migration database.

## Troubleshooting

Start with:

```sh
sudo journalctl -u wrdp.service -b --no-pager
sudo wrdp --config /etc/wrdp/wrdp.ini --diagnose
sudo wrdpctl doctor
sudo ss -ltnp | grep ':3389 '
```

Common failures:

- **Configuration not found:** generate `/etc/wrdp/wrdp.ini` or pass `--config`.
- **TLS load failure:** verify paths, PEM contents, ownership and key mode `0600`.
- **Authentication rejected:** confirm the local account/PAM policy, `allowed_username`, and rate-limit logs.
- **Black display with working input:** check compositor/capture logs, EGFX negotiation and whether software fallback is available; temporarily disable EGFX to isolate bitmap fallback.
- **Hardware encoder unavailable:** verify `/dev/dri/renderD*`, VA driver packages and service permissions. Confirm the journal reports DMA-BUF capture and VA-API encoder creation; WRDP falls back to software only when configured to do so.
- **Clipboard unavailable:** verify the managed compositor exposes data-control and review clipboard policy limits.
- **Audio unavailable:** verify the managed user runtime has a PipeWire socket and inspect RDPSND format negotiation.
- **Resize fails or remains at the prior size:** compare the requested policy, target-user `wlr-randr` result and capture geometry. WRDP keeps the last committed size until an exact matching frame arrives; repeated realization timeouts point to compositor or capture failure.
- **Pointer drift or duplicate clicks:** compare RDP and stream geometry, and confirm Advanced Input became the sole mouse path. The managed capture must not embed a second cursor.
- **Resize ignored:** check `display.allow_resize`, `allowed_resolutions`, the maximum display area and `wlr-randr` access to the managed Wayland socket.
- **Theme missing:** verify `/etc/wrdp/labwc/rc.xml` selects `PlatinumTheme-wrdp-compositor`, the theme exists under `/usr/share/themes`, and the compositor log has no XML parser errors.
- **Stale session:** inspect with `wrdpctl`, stop it cleanly, verify `/run/user/UID/wrdp` belongs to the account, and verify `/run/wrdp/sesman/UID` belongs to root. User-owned legacy state directories are ignored.
- **View-only session accepts input:** verify the daemon loaded the intended configuration and journaled view-only policy. A compliant build creates no input or clipboard backend for that connection.
- **Connection closes after 30 seconds:** if authentication never completed, inspect TLS/PAM latency and client handshake logs. Authenticated session lifetime is controlled separately by `server.session_timeout`.

When reporting a defect, include the exact revision, client name/version, sanitized configuration, diagnostics and relevant journal lines. Never include passwords, private keys, PAM data or clipboard contents.
