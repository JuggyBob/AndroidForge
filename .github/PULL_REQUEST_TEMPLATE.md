## Summary

<!-- What user-visible problem does this solve? Link the issue if one exists. -->

## Behavior

<!-- What changed, and what should a user or agent observe? -->

## Testing

<!-- Commands run, and which emulator/device/app flow was exercised. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo build --workspace --release`
- [ ] Real-device or emulator validation, if device behavior changed

## Checklist

- [ ] The Rust core stays generic; project-specific behavior is in adapters, fixtures, hooks or examples.
- [ ] New behavior has tests, or the PR explains why it cannot be unit-tested.
- [ ] Documentation and `CHANGELOG.md` are updated for notable changes.
- [ ] No secrets, keystores, personal data or private app artifacts are included.
- [ ] Unsupported cases return a structured error with alternatives rather than a stub.

## Notes for reviewers

<!-- Optional: tradeoffs, follow-up work, or evidence paths kept local. -->
