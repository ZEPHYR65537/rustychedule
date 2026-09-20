# Repository guidance

- Preserve the user constraints recorded in `docs/DECISIONS.md`.
- Before changing planning, quota, reset, preference, or progress semantics, read `docs/MATHEMATICS.md`; update it together with implementation and meaningful regression tests.
- Keep each model's quota independent. Tibo is an observed random refresh, never a stored reset card or forecasted quota.
- Token consumption is not task progress. Never infer progress automatically from usage.
- This is a local CLI. Do not introduce runtime network calls, a GUI, credentials, or actual account-reset actions as incidental changes.
- Before committing, run `cargo fmt --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings`.
- Do not commit personal data directories or compiled binaries. `Cargo.lock` is tracked.
- When outputting math in chat or documents, use `$...$` for inline formulas and `$$...$$` for display formulas.
