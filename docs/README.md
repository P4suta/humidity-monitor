# `humidity-monitor`

`humidity-monitor` records temperature, relative humidity, and battery level from the SwitchBot W3400010 Indoor/Outdoor Meter.
It reads advertisements through BlueZ on Linux.
The meter works without a SwitchBot account, hub, or pairing.

## Identify a meter

Power the meter near the Bluetooth adapter and run discovery.
The command prints each address with a complete reading once.
Press Ctrl+C after identifying the meter.

```sh
cargo run --locked -- discover hci0
```

## Record readings

Save the following configuration as `monitor.toml` and replace the example address.
Supply the humidity threshold and confirmation times for the installation.
Durations use seconds.
The database directory must exist and be writable by the service account.

```toml
address = "AA:BB:CC:DD:EE:FF"
label = "Laboratory"
database = "readings.sqlite"
listen = "127.0.0.1:8080"
timezone = "UTC"
retention_days = 365

[policy]
high_above = 70
high_seconds = 1200
recovery_seconds = 120
loss_seconds = 600
```

```sh
cargo run --locked -- serve monitor.toml
```

Open the configured HTTP listener to see the last reading and four history periods.
Use the optional top-level fields `reference = [45, 60]` and `dry_below = 30` to mark the reference band and identify dry readings.
The chart breaks at missing minutes and includes a table of the recorded values.
Long periods group measurements by time and show their range.
The table retains individual minute readings.
[W3C guidance for complex images](https://www.w3.org/WAI/tutorials/images/complex/)

The receiver checks the model, address and data fields before advancing the reception time.
It uses every valid reading for notifications and stores the first reading in each UTC minute for history.
A full missing minute interrupts confirmation of high humidity or its recovery.
Recovery from reception loss requires a valid reading.

## Send notifications

Add an optional `[ntfy]` section with `url` set to the topic URL and `token_file` set to the path of a dedicated publisher token.
Give the publisher write access to the selected topic.
[ntfy access control](https://docs.ntfy.sh/config/#access-control)

SQLite stores each transition and its pending notification together.
Delivery retries keep their original order across restarts.
A restart after the server accepts a message and before the local acknowledgement can repeat a notification.

## Build and test

The build uses stable Rust and a locked dependency graph.
Install Chrome, a matching ChromeDriver, and `dbus-daemon` to run the isolated browser and message bus tests.
Set `CHROMEDRIVER` when the driver is outside the executable search path.

```sh
mise x -- just check
mise x -- just mutation
podman build -f Containerfile -t localhost/humidity-monitor:installed .
```
