## Summary

Describe the user-visible outcome and why this change is needed.

## Validation

List the commands and manual environments used to verify the change.

## Checklist

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes.
- [ ] `cargo test --workspace --locked` passes.
- [ ] Tests cover changed behavior, or the reason they do not is explained.
- [ ] Documentation and configuration examples are updated when needed.
- [ ] No credentials, OAuth material, account identifiers, or private logs are included.
- [ ] IPC/schema compatibility was considered for snapshot or DTO changes.
- [ ] Canonical assets under `crates/cli/assets` and distributable copies under `packaging/` remain synchronized.
- [ ] UI changes include screenshots or a short recording and were checked at popup size.

## Screenshots or migration notes

Add UI evidence, compatibility notes, or write “Not applicable”.
