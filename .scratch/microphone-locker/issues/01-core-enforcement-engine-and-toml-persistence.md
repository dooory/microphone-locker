# 01: Core enforcement engine & TOML persistence

**What to build:** Bootstrap the Rust project and build the testable core through the Application Controller seam (`AudioBackend` trait, in-memory fake backend, and real TOML config persistence). On first run (no config file yet), initialize Default Volume to `80%`, persist the initial config to `%APPDATA%\MicrophoneLocker\config.toml` (or a test-supplied directory), and enforce `80%` on all active Microphones. On subsequent runs, load the saved config and reconcile active Microphones. Whenever an external volume change arrives for a locked Microphone, immediately revert it back to its Target Volume (ignoring sub-0.5% floating-point scalar jitter). When a new active Microphone connects, automatically lock it to follow the Default Volume.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [x] Rust crate builds and `cargo test` passes with controller-level integration tests using an in-memory `AudioBackend` fake and real temporary-directory TOML files
- [x] First run with no existing config file initializes Default Volume to `80%`, saves `config.toml`, enforces `80%` on all active Microphones, and signals that the window should open visible
- [x] Subsequent run with an existing config file loads saved settings, reconciles active Microphones, and signals that the app should start minimized to the tray
- [x] External volume changes on a locked Microphone are immediately reverted to its Target Volume (`0–100%`), while sub-0.5% scalar jitter is ignored
- [x] Newly connected Microphones without a saved Lock Policy are automatically Locked and dynamically follow the Default Volume
- [x] Each Microphone is keyed by its Windows Audio Endpoint ID with its friendly name stored and refreshed when updated
