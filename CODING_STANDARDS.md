# Coding Standards

Review-time judgement calls for `microphone-locker`. Mechanical checks (formatting, `clippy`, compiler warnings, unit/integration tests) belong in CI and `scripts/check.ps1`, not here.

## Domain & Architecture

- Follow `GLOSSARY.md` for all domain-facing identifiers, UI labels, error messages, and test names.
- Follow `docs/adr/` for architectural boundaries: keep OS/Win32 `unsafe` code inside thin adapter modules (`src/windows_*.rs`) behind traits (`AudioBackend`, `StartupRegistry`, etc.) so `AppController` stays testable in memory.

## State & Enforcement (`src/controller.rs`)

- Keep active-microphone enforcement loops DRY through shared helpers on `AppController` rather than repeating `enforce_if_needed` + map-update blocks across command handlers.
- Verify compound state transitions end-to-end in integration tests: custom Target Volume → `Ignored` → un-ignored, `Paused` → resumed (including silent drift while paused), and save-then-reload across restart.
- Guard filesystem helpers against edge-case paths (e.g., bare filenames whose `.parent()` is empty) and test corrupt/missing TOML handling.

## OS Adapters (`src/windows_*.rs`)

- Adapter getters must validate the stored OS value, not merely key or handle existence (e.g., `WindowsStartupRegistry::is_enabled` should confirm the registry value points to the current executable path).
- Keep Win32/COM callback or worker-thread RPC boilerplate behind small internal helpers rather than duplicating channel send/recv error mapping.
