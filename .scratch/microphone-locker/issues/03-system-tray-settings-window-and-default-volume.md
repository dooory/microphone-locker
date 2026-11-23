# 03: System tray icon, settings window & Default Volume control

**What to build:** Add the 100% Rust `egui` (`eframe`) settings window and `tray-icon` integration wired to the Application Controller's view snapshot and command interface. Run as a Windows system tray app: open the settings window automatically on first run, start minimized to the tray on subsequent runs, hide back to the tray when clicking the window's close (X) button, and provide a tray context menu with Show Window and Quit. Display the global Default Volume slider (`0–100%`) and the list of active Microphones with their current/target levels; changing Default Volume immediately updates all active Microphones following Default and persists to `config.toml`.

**Blocked by:** 02: Live Windows Core Audio locking & device hot-plug

**Status:** ready-for-agent

- [ ] App runs in the Windows system tray with left-click (or Show Window menu item) opening/focusing the settings window and Quit exiting cleanly
- [ ] First run (no prior config) opens the settings window automatically; subsequent runs start minimized to the tray
- [ ] Clicking the window's X button hides the window to the tray while volume enforcement continues running
- [ ] Settings window displays all active Microphones with their friendly names and Target Volumes
- [ ] Adjusting the Default Volume slider (`0–100%`) immediately updates and enforces the new level on all active Microphones following Default and persists the change to `config.toml`
- [ ] Controller integration tests verify dynamic Default Volume updates across multiple Microphones and persistence across reload
