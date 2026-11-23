# Microphone Locker

Status: ready-for-agent

## Problem Statement

Windows and third-party applications (such as Discord, Microsoft Teams, Zoom, and games with automatic gain control) silently alter microphone volumes without the user's consent. Users have no built-in way on Windows to lock each Microphone to a chosen volume or ensure that newly connected Microphones automatically start at a safe, consistent level.

## Solution

Microphone Locker is a lightweight, 100% Rust Windows system tray application that strictly enforces each Microphone's configured Target Volume in real time using Windows Core Audio notifications. Every newly discovered active Microphone automatically inherits a configurable Default Volume until the user assigns a custom Target Volume or marks it Ignored. The app stays quietly in the system tray, reverts unauthorized volume changes silently and instantaneously, and provides a clean settings window for managing active and disconnected Microphones.

## User Stories

1. As a Windows user, I want Microphone Locker to run quietly in the Windows system tray, so that it stays out of my taskbar and daily workflow while continuously protecting my Microphone volumes.
2. As a Windows user, I want any external volume change to a locked Microphone to be immediately reverted back to its Target Volume, so that apps and Windows automatic gain control cannot drift my volume.
3. As a Windows user, I want Microphone Locker to revert external volume changes silently without Windows toast notifications, so that I am not spammed with popups during calls or gaming.
4. As a Windows user, I want Microphone Locker to lock only volume level (`0–100%`) and never touch mute state, so that my hardware mute buttons, meeting app mutes, and push-to-talk workflows continue working normally.
5. As a Windows user, I want Target Volume and Default Volume expressed as whole integer percentages (`0–100%`), so that they match Windows Sound Settings directly.
6. As a Windows user, I want to configure a global Default Volume, so that any newly connected Microphone is automatically set to a sensible level without manual setup.
7. As a Windows user, I want newly discovered active Microphones to be automatically Locked and dynamically follow the Default Volume, so that plugging in a new headset or USB Microphone immediately protects it.
8. As a Windows user, I want changing the Default Volume to immediately update every locked Microphone that is currently following the Default Volume, so that I can adjust my standard level in one place.
9. As a Windows user, I want to assign an individual custom Target Volume to any specific Microphone, so that different Microphones can stay locked at different levels.
10. As a Windows user, I want changing the Default Volume to leave Microphones with a custom Target Volume untouched, so that my per-Microphone overrides are preserved.
11. As a Windows user, I want to reset a Microphone with a custom Target Volume back to following the Default Volume, so that I can remove an individual override easily.
12. As a Windows user, I want each Microphone to be identified by its Windows Audio Endpoint ID while displaying its human-readable friendly name, so that two identical Microphone models remain distinct and recognizable.
13. As a Windows user, I want to mark any specific Microphone (such as a virtual audio cable or NVIDIA Broadcast) as Ignored, so that Microphone Locker leaves it completely untouched.
14. As a Windows user, I want to un-ignore a previously Ignored Microphone and return it to Locked, so that I can re-enable enforcement if my setup changes.
15. As a Windows user, I want Ignored status to persist across app restarts, so that I do not have to re-ignore virtual Microphones every time I reboot.
16. As a Windows user, I want a global temporary Paused toggle accessible from both the tray menu and the settings window, so that I can suspend all enforcement while troubleshooting audio without losing any saved Lock Policies.
17. As a Windows user, I want the system tray icon to visually distinguish between Active and Paused states, so that I can tell at a glance whether enforcement is currently running.
18. As a Windows user, I want the Paused state to automatically reset to active when Microphone Locker restarts, so that I cannot accidentally leave volume protection off permanently.
19. As a Windows user, I want resuming from Paused to immediately reconcile all active locked Microphones back to their Target Volumes, so that any changes made while Paused are corrected right away.
20. As a Windows user, I want Microphone Locker to retain the saved Lock Policy of an unplugged Microphone, so that plugging it back in immediately restores its custom Target Volume or Ignored state.
21. As a Windows user, I want disconnected Microphones with saved settings to appear in a secondary, collapsible Disconnected section in the settings window, so that my active Microphones list stays uncluttered while I can still inspect saved devices.
22. As a Windows user, I want a Forget action on any disconnected Microphone, so that I can permanently remove saved entries for hardware I no longer own.
23. As a Windows user, I want Microphone Locker to reconcile all active locked Microphones on startup and after Windows wakes from sleep, so that volume changes occurring while the app was closed or the PC was asleep are corrected.
24. As a Windows user, I want my settings saved automatically in a human-readable TOML configuration file in `%APPDATA%\MicrophoneLocker\config.toml`, so that my configuration persists reliably and can be inspected or backed up easily.
25. As a Windows user, I want Microphone Locker to open its settings window automatically on first run (when no config file exists yet) with Default Volume initialized to 80% and locking active, so that I immediately see my detected Microphones and can review or adjust them.
26. As a Windows user, I want Microphone Locker to start minimized to the system tray on subsequent launches, so that it does not pop up in my face on Windows login.
27. As a Windows user, I want clicking the window's close (X) button to hide the window back to the system tray rather than exiting, so that I do not accidentally stop volume enforcement.
28. As a Windows user, I want an explicit Quit option in the system tray right-click menu, so that I can cleanly exit Microphone Locker when I want to stop it.
29. As a Windows user, I want Microphone Locker to enforce a single running instance so that launching it a second time focuses the existing window instead of running two competing lockers.
30. As a Windows user, I want an optional Start with Windows toggle in the settings window that registers a per-user startup entry without requiring administrator privileges, so that Microphone Locker launches automatically whenever I sign in.

## Implementation Decisions

### Architectural Shape & Module Boundaries
- **Core Domain & Application Controller**: Owns the in-memory state of active and disconnected Microphones, global Default Volume, temporary Paused state, Lock Policy resolution (Locked following Default, Locked with custom Target Volume, or Ignored), volume normalization/tolerance (`0–100%` integer comparison with sub-0.5% scalar tolerance), and reconciliation logic. Exposes a clean command/event interface and a read-only view snapshot for the UI.
- **Configuration Persistence Module**: Loads and saves settings as TOML at `%APPDATA%\MicrophoneLocker\config.toml` (creating parent directories on first save). Tracks Default Volume, per-Microphone entries (keyed by Windows Audio Endpoint ID with stored friendly name and Lock Policy), and whether a config file existed on startup (to drive first-run window visibility).
- **Audio Backend Trait & Windows Core Audio Adapter** (per ADR-0001 and ADR-0004): A narrow `AudioBackend` trait abstracts enumerating active Microphones, reading/setting a Microphone's scalar volume, and receiving asynchronous audio events (external volume change, Microphone connected, Microphone disconnected, friendly name changed, system wake). The production Windows implementation uses Microsoft's `windows` crate and Core Audio COM callbacks (`IAudioEndpointVolumeCallback` and `IMMNotificationClient`) on a dedicated COM thread. When writing volume via `SetMasterVolumeLevelScalar`, the adapter tags calls with a unique application-owned Event Context GUID and ignores callback notifications carrying that same GUID to prevent feedback loops.
- **Startup Registry Trait & Windows Adapter** (per ADR-0004): A narrow `StartupRegistry` trait abstracts querying, enabling, and disabling auto-start. The production Windows implementation manages a per-user entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` pointing to the current executable path, requiring no elevated privileges.
- **Single-Instance Guard**: Uses a named Windows mutex/IPC signal on launch so a second invocation detects the running instance, signals it to show and focus its window, and exits immediately.
- **UI & System Tray Presentation Layer** (per ADR-0002): Built 100% in Rust using `egui` (`eframe`) and `tray-icon`. Renders the Application Controller's view snapshot: global Default Volume slider, global Paused toggle, Start with Windows toggle, Active Microphones list (name, current/target volume slider, Follow Default vs. Custom indicator with Reset to Default, Ignored toggle), and a collapsible Disconnected Microphones section with a Forget button. Clicking the tray icon opens/focuses the window; right-clicking opens a context menu with Show Window, Pause/Resume Locking, and Quit. Clicking the window's X button hides the window to the tray rather than terminating the process. The tray icon visually swaps between distinct Active and Paused icons.

### Key Domain & Edge-Case Rules
- **Device Identity** (per ADR-0003): Microphones are keyed canonically by Windows Audio Endpoint ID. Stored friendly names are refreshed whenever Windows reports a newer name for an existing Endpoint ID.
- **Volume Precision & Tolerance**: User-facing Target Volume and Default Volume are integer percentages `0..=100`. Scalar values from Core Audio (`0.0..=1.0`) are compared against the target scalar with a `0.005` (`0.5%`) tolerance so floating-point quantization in Windows audio drivers does not cause endless revert loops.
- **Dynamic Default Inheritance**: A Microphone without an explicit custom Target Volume stores a "Follow Default" Lock Policy (not a copied number), so updating Default Volume immediately updates and enforces the new level on all active Microphones following Default.
- **Paused State**: Stored only in memory (never persisted to `config.toml`). While Paused, incoming external volume-change events are ignored for enforcement, though device connect/disconnect events still update the active/disconnected lists. Unpausing immediately reconciles all active locked Microphones to their Target Volumes.
- **First-Run vs. Subsequent Runs**: When no config file exists at startup, the controller initializes Default Volume to `80%`, saves the initial config, enforces `80%` on active Microphones, and signals the UI to open visible. When a config file already exists, the UI starts hidden in the tray.
- **System Wake Reconciliation**: On Windows power-resume events (and on initial startup), the controller re-enumerates active Microphones and enforces Target Volumes on all active locked Microphones.

## Testing Decisions

### What Makes a Good Test
- Tests verify **external observable behavior** through the Application Controller's public command and event interface, never internal helper methods or private data structures.
- Tests should remain stable across internal refactorings as long as Microphone Locker's user-facing rules and persisted config format behave identically.

### Primary Testing Seam & Scope
- **Single Primary Seam — Application Controller**: All domain and persistence behavior is tested through the Application Controller wired to:
  - an in-memory **fake `AudioBackend`** (recording volume-set calls and injecting simulated connect, disconnect, wake, and external volume-change events),
  - an in-memory **fake `StartupRegistry`** (verifying enable/disable/query behavior without modifying the host Windows Registry), and
  - **real TOML config files in a temporary directory** (exercising real first-run creation, serialization, corrupt/missing file handling, and restart reload).
- **Behavior Suites Covered**:
  - First-run initialization (`80%` Default Volume, initial config creation, window-open signal) vs. returning startup (starts minimized, loads persisted Lock Policies).
  - External volume-change snap-back for Locked Microphones, tolerance handling for sub-0.5% scalar jitter, and no-op for Ignored Microphones.
  - Dynamic Default Volume updates propagating to Microphones following Default while preserving custom Target Volumes, plus resetting a custom Microphone back to Default.
  - Device connect/disconnect lifecycle: new Microphone auto-locked to Default Volume, unplugged Microphone moving to Disconnected with preserved Lock Policy, replugged Microphone immediately restored, and Forget permanently removing a disconnected entry.
  - Global Paused toggle suspending enforcement, resetting to active across restarts, and reconciling all active locked Microphones upon resume.
  - Startup and wake-from-sleep reconciliation.
  - Start with Windows toggle delegating cleanly to `StartupRegistry`.
- **Outside the Automated Seam**: The thin `egui`/`tray-icon` presentation layer and the thin Windows Core Audio COM / Registry adapters are verified via manual smoke testing on Windows.

### Prior Art
- This is a greenfield Rust repository with no existing test suite; these controller-level integration tests establish the testing standard for the codebase.

## Out of Scope

- Locking or managing microphone mute/unmute state.
- Controlling audio output/playback devices (speakers, headphones).
- Per-application audio routing, per-app mixer volumes, or per-Microphone audio processing/DSP (noise suppression, EQ, compression).
- Non-Windows operating systems (macOS, Linux).
- Automatic USB-port-agnostic hardware fingerprinting beyond Windows Audio Endpoint ID.
- Windows toast notifications or audible alerts on volume snap-back.
- Auto-updater, installer wizard (MSI/MSIX), or code-signing pipeline.

## Further Notes

- Canonical domain terminology is defined in `GLOSSARY.md` (**Microphone**, **Target Volume**, **Default Volume**, **Lock Policy**, **Ignored**, **Paused**).
- Key architectural decisions are recorded in `docs/adr/` (ADR-0001 through ADR-0004).
- This project is also intended as a hands-on Rust learning vehicle using TDD and idiomatic Rust patterns (traits, safe FFI boundaries, clean module separation).
