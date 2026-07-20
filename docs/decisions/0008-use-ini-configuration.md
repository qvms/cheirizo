# ADR 0008: Use INI configuration files

**Status:** accepted
**Date:** 2026-07-19

## Decision

WRDP uses INI files for server and session-manager configuration. `wrdp.ini` holds listener, security, display, channel and encoder settings. `wrdp-sesman.ini` holds per-user compositor lifecycle settings.

The configuration loader supports sectioned `key=value` files, comments, typed validation and selected environment overrides. Breaking changes increment `config_version` and use explicit migration or a clear validation error.

## Why

WRDP follows the xrdp/sesman operating model. Administrators who already manage xrdp know its sectioned INI files, so keeping that format makes WRDP configuration immediately familiar and keeps server/session-manager settings easy to compare.

INI also fits small files under `/etc` and keeps generated examples readable. Changing formats would add migration work without improving runtime behavior.

## Consequences

- Generated configuration and examples use INI syntax.
- Packaging installs examples under `/etc/wrdp/` without overwriting local changes.
- Environment variables are overrides, not a second complete configuration model.
- New settings belong to the module that consumes them and must include validation and generated documentation.
- Deprecated keys are handled through `config_version`, migration or explicit rejection.

## Alternatives considered

- **TOML:** rejected because changing formats would add migration work without solving a current problem.
- **Environment variables only:** rejected because encoder, channel and session settings need a readable persistent file.
- **JSON or YAML:** rejected because comments and hand editing are important for host configuration.
