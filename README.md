# Microphone Locker

Voice apps, games, and Windows updates sometimes change your microphone volume without asking. Microphone Locker sits in the Windows system tray and snaps the volume back whenever something changes it.

By default, every microphone follows a global default volume, including new ones you plug in. You can also give individual microphones their own target volume or mark them as ignored. If you need to adjust a level temporarily, pause locking from the tray menu. Settings live in `%APPDATA%\MicrophoneLocker\config.toml`.

## Setup

Microphone Locker runs on Windows.

### Download

Download `microphone-locker.exe` from the [latest GitHub release](https://github.com/dooory/microphone-locker/releases/latest) and run it. If you plan to turn on "Start with Windows" in the app window, move the executable to a permanent folder first so Windows can find it on sign-in.

### Build from source

To build it yourself, install:

- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain, 2024 edition)
- Visual Studio C++ Build Tools (the "Desktop development with C++" workload, for `link.exe`)

Clone the repo and run it with Cargo:

```powershell
git clone https://github.com/dooory/microphone-locker.git
cd microphone-locker
cargo run --release
```

To build a standalone executable without running it:

```powershell
cargo build --release
```

That puts the binary at `target\release\microphone-locker.exe`.

## Contributing

Issues and pull requests are welcome on [GitHub](https://github.com/dooory/microphone-locker).

Before opening a pull request, run the check script from the repo root:

```powershell
.\scripts\check.ps1
```

It runs `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`, and loads the MSVC environment automatically if `link.exe` is not on your `PATH`.

If you plan to change code, check these docs first:

- [GLOSSARY.md](GLOSSARY.md) for the domain terms used in the code and UI
- [CODING_STANDARDS.md](CODING_STANDARDS.md) for architecture and review conventions
- [docs/adr/](docs/adr/) for the main design decisions
