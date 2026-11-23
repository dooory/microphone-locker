# 02: Live Windows Core Audio locking & device hot-plug

**What to build:** Implement the real Windows Core Audio adapter behind the `AudioBackend` trait using Microsoft's `windows` crate and wire it into a runnable binary. Enumerate active Windows audio capture endpoints on startup, enforce their Target Volumes, listen for real-time volume changes via `IAudioEndpointVolumeCallback`, and listen for device arrivals/removals and property updates via `IMMNotificationClient`. Tag all volume writes with an application-owned Event Context GUID and ignore notifications carrying that GUID to prevent feedback loops. Never modify mute/unmute state.

**Blocked by:** 01: Core enforcement engine & TOML persistence

**Status:** ready-for-agent

- [ ] Launching the binary on Windows enumerates all active Microphones and enforces their Target Volumes via Windows Core Audio without altering mute state
- [ ] Changing a locked Microphone's volume in Windows Sound Settings or an external app immediately snaps it back to its Target Volume without feedback loops (verified via Event Context GUID filtering)
- [ ] Plugging in a new Microphone while running automatically detects it and sets it to the Default Volume
- [ ] Unplugging a Microphone while running cleanly detaches its volume callback without crashing or leaking COM callbacks
- [ ] All `unsafe` Win32/COM code remains isolated inside the Windows audio adapter behind `AudioBackend`
