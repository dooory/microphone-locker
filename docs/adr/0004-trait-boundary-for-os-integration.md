# Trait boundary for Windows OS integration

Even though Microphone Locker targets Windows exclusively, all Windows Core Audio and startup Registry interactions are isolated behind narrow Rust traits (e.g., `AudioBackend`, `StartupRegistry`). This keeps `unsafe` COM/Win32 code confined to a thin adapter layer and allows the entire domain engine—Lock Policies, Default Volume inheritance, Pause/Ignore rules, and config persistence—to be tested deterministically in memory without touching live audio hardware.

## Considered Options

- **Trait boundary with in-memory fakes (chosen)**: Fast, deterministic TDD for all domain rules and clean isolation of `unsafe` Windows code, at the cost of a thin abstraction layer.
- **Direct Windows API calls in domain logic**: Less upfront indirection, but makes automated testing dependent on real hardware and OS audio state.
