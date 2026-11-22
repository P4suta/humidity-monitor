---
date: 2026-11-22
---

# Link the container receiver as a static position-independent executable

## Status

Proposed.

## Context

On ARM64 Linux, the waiting receiver used about 4.7 MiB of executable pages and 1.6 MiB of shared libraries.
Its heap used less than 0.2 MiB.

## Decision

Link the container executable with static glibc and preserve position independence.
Keep the Debian runtime image for its nonroot account and host name resolution.
Use size optimization and link-time optimization for release builds.
[Rust linkage](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes) and [Cargo release profiles](https://doc.rust-lang.org/cargo/reference/profiles.html).

## Alternatives

The dynamically linked size-optimized executable still mapped the shared C runtime.
The official ARM64 musl build used less resident memory, but its executable had a fixed load address.

## Consequences

A C runtime update requires rebuilding the executable.
Revisit the link choice if name resolution or annual history latency regresses.

## Confirmation

Check the executable type and program headers and run the shipped receiver on Linux.
Compare resident memory during reception and history requests, and deliver a notification to a named service after a restart.
