# 07: Start with Windows toggle, wake reconciliation & single-instance guard

**What to build:** Complete Windows desktop polish end-to-end: add the `StartupRegistry` trait and per-user Windows `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` adapter wired to a "Start with Windows" toggle in the settings window (requiring no admin privileges); reconcile all active locked Microphones automatically when Windows wakes from sleep; and enforce a single running instance so launching the executable a second time focuses the existing window and exits cleanly.

**Blocked by:** 03: System tray icon, settings window & Default Volume control

**Status:** ready-for-agent

- [ ] "Start with Windows" toggle in the settings window accurately reflects and updates the per-user Registry `Run` entry without requiring administrator privileges
- [ ] Controller integration tests verify Start with Windows query/enable/disable behavior through an in-memory `StartupRegistry` fake
- [ ] System wake-from-sleep events trigger full reconciliation of all active locked Microphones (verified via controller tests and Windows power-resume handling)
- [ ] Launching a second instance of Microphone Locker detects the running instance, brings the existing settings window to the foreground, and exits immediately without creating a competing locker
