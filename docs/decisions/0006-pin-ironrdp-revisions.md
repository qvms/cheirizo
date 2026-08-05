# ADR 0006: Pin tested IronRDP revisions

**Status:** accepted
**Date:** 2026-07-19

## Decision

WRDP will consume the IronRDP crates needed by the server from one tested Git revision. Moving branch specifications are replaced with immutable commit revisions.

Changes that are useful outside WRDP should remain suitable for upstreaming to IronRDP. WRDP keeps local policy and desktop integration outside protocol crates.

## Why

WRDP depends on server and EGFX changes that are not all available in released crates. A moving branch makes clean builds change over time and can split the IronRDP crate graph across incompatible revisions.

One revision gives Cargo a coherent protocol stack and makes build results repeatable.

## Consequences

- Updating IronRDP is an explicit change with protocol and client tests.
- Every IronRDP Git dependency points to the same commit.
- The lockfile is committed for applications and release builds.
- Release notes and `THIRD_PARTY.md` identify the IronRDP revision.
- WRDP's current IronRDP stack requires Rust 1.94 or newer.
- CI rejects branch-based IronRDP dependency specifications.

## Alternatives considered

- **Use the latest branch head:** rejected because branch heads move.
- **Fork protocol behavior permanently into WRDP:** rejected because the protocol core should stay shared with IronRDP.
- **Wait for every crate to reach crates.io:** rejected because it would block the server while required EGFX work remains unreleased.
