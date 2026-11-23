# 04: Per-Microphone custom Target Volume, Reset to Default & Ignored policy

**What to build:** Add full per-Microphone Lock Policies across the Application Controller, TOML config, and `egui` settings window. Allow the user to assign an individual custom Target Volume (`0–100%`) to any Microphone (immune to future Default Volume changes), reset a custom Microphone back to following the Default Volume, or persistently mark any Microphone as Ignored so Microphone Locker leaves its volume completely untouched across restarts (and allow un-ignoring it back to Locked).

**Blocked by:** 03: System tray icon, settings window & Default Volume control

**Status:** ready-for-agent

- [ ] Setting a custom Target Volume on a Microphone immediately enforces that level, persists the custom Lock Policy to `config.toml`, and is unaffected when Default Volume changes
- [ ] Clicking Reset to Default on a custom Microphone switches its Lock Policy back to following Default Volume, enforces the current Default Volume, and persists the change
- [ ] Marking a Microphone as Ignored stops enforcing its volume immediately, persists across restarts, and ignores external volume changes on that Microphone
- [ ] Un-ignoring an Ignored Microphone returns it to Locked and immediately enforces its Target Volume
- [ ] Controller integration tests cover custom Target Volume, Reset to Default, Ignored/un-ignored transitions, and TOML round-trip persistence
