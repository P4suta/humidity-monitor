---
date: 2026-11-22
---

# Store minute history and pending notifications in redb

## Status

Accepted.

## Context

An unattended receiver must recover the last reception and pending notifications after power loss.
The first sample in a minute stays fixed until its retention period ends.

## Decision

Use typed redb tables for samples, receiver state and pending notifications.
Commit each reception and its notification transitions together with immediate durability.
Read the dashboard from one snapshot.
Import a SQLite snapshot into a private file and publish the completed database without overwriting existing files.
[redb storage design](https://github.com/cberner/redb/blob/master/docs/design.md) and [SQLite isolation](https://www.sqlite.org/isolation.html).

## Alternatives

SQLite already supports durable transactions and SQL chart aggregation.
The redb tables bring the minute key and sample layout into Rust.
Fjall requires application codecs for keys and values.

## Consequences

The receiver must group samples and split chart lines at gaps in Rust.
An upgrade needs space for the source and destination files together.
Revisit the storage choice if reading a year of samples delays recording.

## Confirmation

Close the writer abruptly around a commit and reopen the same database.
Compare ranges, missing minutes, expiry and pending notifications before and after an upgrade.
Test storage failures and concurrent year scans.
