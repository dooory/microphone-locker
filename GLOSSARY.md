# Microphone Locker

A Windows system tray application that locks microphone input volumes to configured levels and automatically applies a default volume to newly connected microphones.

## Language

**Microphone**:
A Windows audio capture endpoint (physical or virtual), uniquely identified by its Windows Audio Endpoint ID.
_Avoid_: Input, recording device, audio source

**Target Volume**:
The desired volume level (whole integer percentage, `0–100%`) enforced on a locked Microphone.
_Avoid_: Gain, level, volume setting

**Default Volume**:
The fallback Target Volume dynamically inherited by locked Microphones that do not have a custom Target Volume assigned.
_Avoid_: Initial volume, global volume, baseline

**Lock Policy**:
The persistent per-Microphone rule determining whether the Microphone is Locked (following the Default Volume or a custom Target Volume) or Ignored.
_Avoid_: Mode, profile, rule

**Ignored**:
A persistent per-Microphone state in which Microphone Locker does not monitor or alter the Microphone's volume.
_Avoid_: Excluded, disabled, muted, unlocked

**Paused**:
A temporary global state that suspends volume enforcement across all Microphones without modifying their Lock Policies; resets to active when the app restarts.
_Avoid_: Stopped, disabled, unlocked
