# Minimal Platinum desktop

WRDP's default managed session is a small Wayland desktop, not a general desktop environment. It combines the existing Platinum window theme with a top taskbar, notifications, file management, an application launcher and an offline-safe wallpaper.

## Components

| Role | Component |
|---|---|
| Window management | `wrdp-compositor` |
| Top taskbar and tray | Waybar |
| File manager | Thunar |
| File services and thumbnails | GVFS, GVFS backends and Tumbler |
| Notifications | Mako |
| Launcher | Fuzzel |
| Wallpaper | swaybg |
| Terminal | Foot |

The minimal profile does not install or start `xfce4-session`, `xfce4-panel`, `xfwm4`, `xfdesktop` or another desktop-icon owner. A future XFCE profile must be curated as an explicit alternative and must not start duplicate panel, wallpaper, notification or window-management processes.

## Provisioning

The repository vendors `ground-init` under `vendor/ground-init`. Its YAML files are trusted administrator input.

Provision packages and system-owned files from the repository root:

```sh
sudo make provision-system
```

Provision one local account:

```sh
sudo make provision-user PROVISION_USER=rui
```

The system pass installs packages and copies canonical files into `/etc/wrdp`, `/usr/lib/wrdp` and `/usr/share/themes`. The user pass writes only beneath that account's home directory. Both passes are idempotent and may be repeated after an upgrade.

Automatic first-login provisioning is deliberately disabled in the first version. A missing `~/.config/wrdp/provisioned-v1` marker is diagnostic information, not permission for the network-facing daemon to run package or home-directory setup.

## Taskbar

The top Waybar contains:

- launcher, Files and Terminal controls;
- active task buttons;
- StatusNotifier tray;
- clock;
- Reconfigure and Disconnect session actions.

The style uses the existing Platinum palette, square edges and Charcoal when installed. It falls back to Liberation Sans.

## Wallpaper

The default is solid `#bdbab4` and requires no network access. Per-user settings live at `~/.config/wrdp/wallpaper.conf`.

An optional Bing image can be fetched explicitly:

```sh
/usr/lib/wrdp/wrdp-wallpaper --refresh-bing
sed -i 's/^mode=.*/mode=bing/' ~/.config/wrdp/wallpaper.conf
```

The fetcher accepts only Bing HTTPS image URLs, validates content type, caps downloads at 20 MiB and atomically replaces `~/.cache/wrdp/wallpapers/bing-current.jpg`. Session startup never performs network access; a missing cache falls back to grey.

## Package tiers

The default package list is in `ground-init.system.yaml`. Thunar volume management, archive plugins, a Polkit agent and XFCE session management are not installed until a concrete workflow requires them.

## Rollback

Before reprovisioning a deployed host, back up:

- `/etc/wrdp/labwc`;
- `/etc/wrdp/waybar`;
- `/etc/wrdp/mako`;
- `/etc/wrdp/wallpaper`;
- `/usr/lib/wrdp/wrdp-desktop-action`, `wrdp-desktop-session` and `wrdp-wallpaper`;
- the existing Platinum theme;
- affected user files under `~/.config`.

Restore those paths and restart the managed compositor session. Package removal is optional; an unused Thunar or Mako package does not affect WRDP when it is absent from autostart.

## Acceptance and memory measurements

The Debian 13 deployment on LXC 121 was accepted at `1024x768`, `1280x960` and `1376x960`. Each authoritative root-owned session record reported the requested geometry and one active client. The desktop had exactly one Waybar, Mako and swaybg instance; no `xfwm4`, `xfce4-panel`, desktop-icon owner or automatically launched terminal was present. The visible controls launched Thunar, Foot and Fuzzel; Alt-Tab/taskbar state, Mako notifications, StatusNotifier ownership, GVFS, Tumbler, reconnect and the Session Disconnect action were also exercised. Disconnect left the socket active and no desktop processes.

Run `scripts/measure-desktop-memory.py` as root to include PSS for both the root daemon and user session processes. Reports sample cgroup memory, process RSS/PSS and per-process-name totals without recording process arguments or environment values.

The table below uses five samples at two-second intervals. The daemon baseline held one no-byte, unauthenticated TCP connection open so socket activation kept only the root daemon alive; it measured **2.07 MiB cgroup** (σ 0), **11.15 MiB PSS** and **14.50 MiB RSS**. Its report embeds socket/service state, root PID/UID, connection count, zero application bytes, no authentication and zero desktop processes. Reconnect reports embed pre-disconnect, zero-client and post-reconnect timestamps plus the unchanged authenticated compositor PID, kernel start ticks, boot ID, UID and PGID. Every desktop row had a complete PSS/RSS process set. Deltas are relative to the daemon baseline.

| Geometry | Scenario | Cgroup MiB ± σ | Δ cgroup MiB | PSS MiB | Δ PSS MiB | RSS MiB |
|---|---|---:|---:|---:|---:|---:|
| 1024x768 | idle | 135.7 ± 0.28 | +133.6 | 199.8 | +188.7 | 322.8 |
| 1024x768 | Thunar | 166.4 ± 13.92 | +164.3 | 240.4 | +229.3 | 455.3 |
| 1024x768 | notification/tray | 158.1 ± 0.25 | +156.1 | 228.4 | +217.2 | 419.8 |
| 1024x768 | reconnect | 152.6 ± 0.05 | +150.5 | 215.8 | +204.6 | 338.8 |
| 1280x960 | idle | 164.1 ± 0.13 | +162.0 | 221.8 | +210.6 | 344.9 |
| 1280x960 | Thunar | 194.5 ± 16.43 | +192.4 | 262.4 | +251.3 | 477.5 |
| 1280x960 | notification/tray | 185.2 ± 0.27 | +183.1 | 250.3 | +239.2 | 441.9 |
| 1280x960 | reconnect | 170.8 ± 0.21 | +168.8 | 228.3 | +217.1 | 351.3 |
| 1376x960 | idle | 168.2 ± 0.19 | +166.1 | 226.4 | +215.2 | 349.8 |
| 1376x960 | Thunar | 198.2 ± 14.66 | +196.1 | 267.2 | +256.1 | 482.1 |
| 1376x960 | notification/tray | 189.9 ± 0.11 | +187.8 | 255.0 | +243.9 | 446.3 |
| 1376x960 | reconnect | 175.6 ± 0.14 | +173.6 | 233.2 | +222.1 | 356.4 |

PSS is the preferred process-tree total; RSS double-counts shared pages. Cgroup memory includes allocator state, page cache and kernel-accounted memory; `memory.peak` is cumulative for the service lifetime, not scenario-local. GPU buffers are not fully represented by either value. The larger Thunar σ values reflect GVFS/Tumbler activation settling during the five-sample window rather than duplicate process starts.

The deployment rollback bundle is `/var/backups/wrdp/20260809T153437Z-desktop`; it records absent-before paths, package versions, service state and hashes of `/usr/bin/wrdp` and `/usr/lib/wrdp/wrdp-compositor`.
