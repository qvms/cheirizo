# ground-init provenance

WRDP vendors `ground-init.py` from:

- Repository: <https://github.com/rcarmo/ground-init>
- Revision: `f19e0a8f100e988e99965ea480384a955ce031d2`
- Upstream date: 2026-04-20
- Licence: MIT; see `LICENSE`
- Upstream script SHA-256: `1896518d711149a81b25a4b1e785f74709dbc00749aff81b748c46a3bf9bb64f`

The WRDP copy adds three non-shell, standard-library handlers:

- `directories` creates directories and applies optional modes.
- `copy_files` copies individual files and applies optional modes.
- `copy_trees` merges a source tree into a destination tree.

WRDP configuration is trusted administrator input. The profiles deliberately avoid the upstream `runcmd`, repository-build and package-upgrade handlers.

To update the vendored copy, inspect the new upstream revision, replace `ground-init.py` and `LICENSE`, reapply the three local handlers, update this file, and run:

```sh
python3 -m unittest vendor/ground-init/test_ground_init.py
python3 scripts/test_desktop_provisioning.py
```
