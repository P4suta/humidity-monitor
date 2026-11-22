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
Set `database` to a filesystem path for persistent history.
The database directory must exist and be writable by the service account.

```toml
address = "AA:BB:CC:DD:EE:FF"
label = "Laboratory"
database = "readings.redb"
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
Long periods group measurements into at most 300 time intervals.
The chart keeps the first and last continuous runs in each interval.
When several runs lie between them, the chart marks their minimum and maximum at the recorded times.
The table retains individual minute readings.
[W3C guidance for complex images](https://www.w3.org/WAI/tutorials/images/complex/)

The receiver checks the model, address and data fields before advancing the reception time.
It uses every valid reading for notifications and stores the first reading in each UTC minute for history.
A full missing minute interrupts confirmation of high humidity or its recovery.
Recovery from reception loss requires a valid reading.

## Upgrade and restore history

Stop the receiver before upgrading.
On first start, the receiver imports a neighboring `readings.sqlite` file into `readings.redb`.
An existing configuration with a `.sqlite` filename opens the same basename with the `.redb` extension.
For SQLite files with other names, the receiver appends `.redb` to the full filename.
The import needs disk space for both databases.
The receiver keeps the SQLite source and publishes the redb file after importing the complete history and pending notifications.
Restarting after an interrupted import repeats the import from the source.

To back up history, stop the receiver, copy the redb file to separate storage and restart the receiver.
To restore, stop the receiver and point `database` to a separate copy of the backup.
Start the receiver with the original meter address and check the last reception and history on the dashboard.
Archive the original SQLite database together with its `-wal` file when a write-ahead log is present.

## Send notifications

Add an optional `[ntfy]` section with `url` set to the topic URL and `token_file` set to the path of a dedicated publisher token.
Give the publisher write access to the selected topic.
[ntfy access control](https://docs.ntfy.sh/config/#access-control)

A redb transaction stores each transition and its pending notification together.
Delivery retries keep their original order across restarts.
A restart after the server accepts a message and before the local acknowledgement can repeat a notification.

## Build and test

The build uses stable Rust and a locked dependency graph.
Install Chrome, a matching ChromeDriver, and `dbus-daemon` to run the isolated browser and message bus tests.
Set `CHROMEDRIVER` when the driver is outside the executable search path.

```sh
mise x -- just check
mise x -- just mutation
mise x -- just storage-mutation
mise x -- just storage-endurance
podman build -f Containerfile -t localhost/humidity-monitor:installed .
```
