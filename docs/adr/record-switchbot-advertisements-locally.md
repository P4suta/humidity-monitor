---
date: 2026-11-22
---

# Record meter advertisements locally

## Status

Proposed.
The [redb storage decision](store-meter-history-in-redb.md) supersedes the SQLite choice.

## Context

Local recording needs to continue when the internet is unavailable.
The W3400010 broadcasts humidity, temperature and battery level in its Bluetooth advertisements.
[SwitchBot packet format](https://github.com/OpenWonderLabs/SwitchBotAPI-BLE/blob/latest/devicetypes/meter.md#outdoor-temperaturehumidity-sensor)

## Decision

Decode advertisements from one registered address in Rust using the BlueZ system bus.
Keep minute samples, the last valid reception, and pending notifications in SQLite.
Read thresholds and notification destinations from deployment configuration.
[BlueZ Adapter API](https://bluez.readthedocs.io/en/latest/adapter-api/)

## Alternatives

A SwitchBot hub can forward readings to a cloud account.
The extra hardware and internet dependency conflict with local recording.

## Consequences

The host needs Bluetooth and access to the system bus.
Reconsider receiver placement if radio outages recur.

## Confirmation

Replay the public advertisement, advance the clock across notification thresholds and restart the service with the same database.
Confirm radio coverage at the installation site during commissioning.
