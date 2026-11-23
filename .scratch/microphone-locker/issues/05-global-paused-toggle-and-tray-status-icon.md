# 05: Global temporary Paused toggle & tray status icon

**What to build:** Add the global temporary Paused state accessible from both the system tray right-click menu and the settings window. While Paused, suspend all volume enforcement without altering any Microphone's saved Lock Policy (while continuing to track Microphone connect/disconnect events). Visually distinguish Active vs. Paused in the tray icon. Never persist Paused to `config.toml` so restarts always resume active enforcement. Resuming from Paused immediately reconciles all active locked Microphones back to their Target Volumes.

**Blocked by:** 03: System tray icon, settings window & Default Volume control

**Status:** ready-for-agent

- [ ] Global Pause/Resume toggle is available in both the tray context menu and the settings window and stays in sync
- [ ] While Paused, external volume changes on locked Microphones are not reverted, and saved Lock Policies remain unchanged
- [ ] Resuming from Paused immediately reconciles all active locked Microphones back to their Target Volumes
- [ ] Tray icon visibly switches between distinct Active and Paused icons
- [ ] Paused state is never written to `config.toml` and always resets to active on app restart
- [ ] Controller integration tests verify pause suspension, resume reconciliation, and restart reset behavior
