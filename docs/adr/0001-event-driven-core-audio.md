# Event-driven Windows Core Audio enforcement

We use Windows Core Audio COM callbacks (`IAudioEndpointVolumeCallback` and `IMMNotificationClient`) rather than a polling timer to detect volume changes and device arrivals/removals, paired with a reconciliation pass on startup and system wake. This gives instant snap-back and near-zero idle CPU usage while serving as a real-world Rust/Win32 FFI learning exercise, at the cost of extra COM setup and thread-safety discipline.

## Considered Options

- **Event-driven Core Audio callbacks (chosen)**: Instant reversion, minimal CPU overhead, idiomatic Windows audio integration.
- **Timer-based polling**: Simpler implementation, but wakes continuously while idle and allows brief windows where external volume changes persist.
