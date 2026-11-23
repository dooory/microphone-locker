# Identify Microphones by Windows Audio Endpoint ID

Each Microphone is keyed canonically by its Windows Audio Endpoint ID, with its human-readable friendly name stored alongside it for display and config readability. Endpoint IDs cleanly distinguish identical microphone models and remain stable when replugging into the same port, though moving a USB microphone without a unique hardware serial to a different port may register as a new Microphone and inherit the Default Volume.

## Considered Options

- **Windows Audio Endpoint ID (chosen)**: Exact OS-level identity, handles duplicate hardware cleanly.
- **Friendly device name**: Survives USB port changes more often, but breaks when two identical devices exist or when users rename devices in Windows Sound Settings.
