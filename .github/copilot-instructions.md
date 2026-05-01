# GitHub Copilot Instructions — kinetic-audio

`kinetic-audio` is a Rust library crate for cross-platform game audio with native (`cpal`) and browser (`Web Audio`) backends.
Generated changes should preserve the crate's cross-platform API and keep behavior consistent across `CpalBackend`, `WebAudioBackend`, and `NullBackend` wherever the architecture allows it.

---

## CI Readiness

- Before handing work off, use the local commands that represent the supported targets and feature sets:
  - `cargo test`
  - `cargo clippy --all-targets --all-features`
  - `cargo build --features symphonia`
  - `cargo check --target wasm32-unknown-unknown --features "web-backend symphonia"`
- If a change affects examples or docs, make sure the documented API still matches the code.
- If a target or feature combination cannot be verified locally, say so explicitly.

---

## Code Quality

- Write production-quality Rust: no placeholder logic, no `todo!()` in shipping code, no silent fallbacks that hide broken audio behavior.
- Prefer `thiserror`-based domain errors and `?` for propagation.
- Keep `cargo clippy` clean. If an `#[allow(...)]` is unavoidable, keep it narrow and explain why.
- Format code with `rustfmt`.
- Avoid `unsafe`. If it becomes necessary, every block must have a `// SAFETY:` explanation.

---

## Cross-Platform Audio Expectations

- `AudioManager` is the user-facing contract. Changes to playback, spatial audio, sprites, tweening, buses, or effects must be reviewed for:
  - native backend behavior
  - Web Audio backend behavior
  - `NullBackend` test behavior
- Do not claim feature parity in docs unless the feature really works on both native and WASM.
- When backend parity is not practical, document the limitation clearly in `README.md` instead of implying support.
- Preserve the current API shape when possible; prefer filling behavior gaps over adding parallel APIs.

---

## Testing

- Add or update tests for non-trivial behavior changes.
- Prefer unit tests in the module for manager/backend logic and integration tests in `tests/` for user-facing flows.
- `NullBackend` is the default choice for deterministic playback tests; use it to assert routing, tweening, finished-voice handling, and spatial recomputation.
- Avoid tests that require a real audio device.

---

## Documentation

- All public items should have `///` docs.
- When public API signatures change, update:
  - crate-level docs in `src/lib.rs`
  - `README.md`
  - examples in `examples/` when applicable
- Keep the README honest about feature flags and backend-specific limitations.

---

## Publishing

- This crate is intended for crates.io publishing. Keep `Cargo.toml` metadata, README examples, and feature descriptions release-ready.
- Do not publish automatically from code changes; use `scripts/publish-crates-io.sh` and keep manual confirmation in the loop.
