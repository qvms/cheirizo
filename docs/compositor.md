# Managed compositor

WRDP starts one headless Wayland compositor for each authenticated user. The compositor is a modified labwc 0.8.3 program under `vendor/wrdp-compositor/`.

It runs independently of any GNOME or KDE session on the host. The session manager supplies the user's runtime directory, environment, supplementary groups, render node, and `/etc/wrdp/labwc` configuration directory, then waits for the Wayland socket. After authentication, WRDP applies the negotiated desktop size to `HEADLESS-1` before it creates the capture stream, so a reused session does not retain the previous client's geometry.

## Server interfaces

WRDP uses standard Wayland/wlroots protocols exposed by the compositor:

- `wlr-screencopy` or `ext-image-copy-capture` for frames;
- virtual keyboard, virtual pointer or EIS for input;
- `ext-data-control` or `wlr-data-control` for clipboard;
- output-management protocols for desktop size changes.

The managed production path receives screencopy frames through an in-process channel after connecting to the user's Wayland socket. It does not need a GNOME/KDE portal or a PipeWire video stream. Capture prefers DMA-BUF and falls back to SHM. PipeWire supplies redirected audio and remains available to the separate portal path.

## Default session UI

The managed session starts a small terminal and, when installed, a minimal
[Waybar](https://github.com/Alexays/Waybar) taskbar. WRDP's default Waybar
configuration uses only `wlr/taskbar` and `clock`; it does not poll network,
audio, power or tray services. The bar is optional: compositor startup checks
for the `waybar` executable and continues normally when it is absent.

Install the managed-session defaults with:

```bash
sudo apt install waybar
make install-session-defaults
```

Files are installed under `/etc/wrdp/labwc` and `/etc/wrdp/waybar`. The default window theme is installed as `/usr/share/themes/PlatinumTheme-wrdp-compositor`; the source-controlled `rc.xml` selects it and loads valid `Frame`, `Title` and `Root` mouse contexts.

## Build boundary

Meson builds the compositor as a GPL-2.0-only executable. It is installed at:

```text
/usr/lib/wrdp/wrdp-compositor
```

The MIT Rust binaries start it as a child process; they do not link compositor objects. Upstream and local-change details are in [`vendor/wrdp-compositor/UPSTREAM.md`](../vendor/wrdp-compositor/UPSTREAM.md).

## Local build

The compositor requires wlroots 0.18 and the matching Wayland development packages:

```bash
meson setup build/compositor vendor/wrdp-compositor \
  --buildtype=release \
  -Dxwayland=disabled \
  -Dicon=disabled
meson compile -C build/compositor
```

Xwayland and desktop-entry icon lookup are optional build features. They are not required for the managed headless session. When Xwayland is enabled, it uses the same render node as the compositor.
