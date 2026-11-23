# Pure Rust UI with egui and tray-icon

We build the system tray interface and settings window entirely in Rust using `egui` (`eframe`) and `tray-icon`, rather than splitting the stack with Tauri (HTML/CSS/JS) or limiting the UI to a tray context menu. Keeping 100% of the codebase in Rust directly supports the project's Rust learning goal, delivers real volume sliders and device management, and produces a single lightweight executable.

## Considered Options

- **`egui` + `tray-icon` (chosen)**: 100% Rust, fast iteration, small single binary, full slider/toggle UI.
- **Tauri**: Rich web UI capabilities, but splits the project across Rust and web technologies.
- **Tray menu only**: Minimal dependencies, but awkward for setting arbitrary per-microphone volume percentages.
