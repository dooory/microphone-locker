# 06: Disconnected Microphones section & Forget action

**What to build:** Complete the unplugged Microphone lifecycle across the Application Controller, TOML config, and `egui` settings window. When a configured Microphone is unplugged, retain its saved Lock Policy and move it into a secondary, collapsible Disconnected section in the settings window. Replugging the Microphone immediately moves it back to Active and restores its saved Lock Policy. Provide a Forget button on each disconnected Microphone that permanently removes its saved entry from memory and `config.toml`.

**Blocked by:** 04: Per-Microphone custom Target Volume, Reset to Default & Ignored policy

**Status:** ready-for-agent

- [ ] Unplugging a Microphone with a saved Lock Policy removes it from Active Microphones, retains its Lock Policy, and lists it in a collapsible Disconnected section
- [ ] Replugging a previously disconnected Microphone moves it back to Active and immediately restores its saved custom Target Volume or Ignored state
- [ ] Clicking Forget on a disconnected Microphone permanently deletes its entry from state and `config.toml` (so replugging it later treats it as a newly discovered Microphone following Default Volume)
- [ ] Controller integration tests verify disconnect preservation, replug restoration, and Forget persistence
