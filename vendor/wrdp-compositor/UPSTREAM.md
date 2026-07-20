# Upstream and local changes

This `wrdp-compositor` 0.8.3 snapshot is derived from [labwc 0.8.3](https://github.com/labwc/labwc/releases/tag/0.8.3):

```text
upstream commit: 1fe4797a9d29b5094c9e38c336752d7f57ed593f
license: GPL-2.0-only
```

The tree contains changes made before and during WRDP integration. File-level SPDX, author and adaptation notices remain the record for individual files; for example, the XPM loader credits its later adapter explicitly. The complete compositor remains GPL-2.0-only, with separately licensed artwork listed in `ASSET_LICENSES.md`.

WRDP changes the compositor for headless, per-user remote desktop sessions:

- renames the executable, desktop entry, portal descriptor and configuration paths;
- exposes the standard wlroots screencopy, virtual input and data-control protocols used by the WRDP server;
- selects headless wlroots operation and WRDP-owned session defaults;
- adds configuration and icons for managed sessions;
- adds the `PlatinumTheme-wrdp-compositor` artwork, recreated for WRDP from emulator screenshots rather than copied from labwc;
- removes features and integration paths that are not used by WRDP's desktop contract;
- adjusts capture, input, clipboard and lifecycle behavior for server-owned sessions.

To compare this tree with the recorded upstream release:

```bash
git clone --depth 1 --branch 0.8.3 https://github.com/labwc/labwc.git /tmp/labwc-0.8.3
diff -ru --exclude=.git /tmp/labwc-0.8.3 vendor/wrdp-compositor
```

A compositor update must repeat this comparison, preserve all license notices, and document changes to the frame-export and managed-session interfaces.
