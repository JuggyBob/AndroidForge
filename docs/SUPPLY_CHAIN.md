# Supply chain

`deny.toml` defines the dependency policy checked by CI:

- RustSec advisories, including yanked crates;
- permissive license allowlist;
- wildcard dependency rejection;
- unknown registry and git source rejection.

CI job `supply-chain` runs:

```text
cargo deny check advisories bans licenses sources
```

The policy currently records four unmaintained-advisory exceptions, all with no safe upgrade in
this dependency graph: `RUSTSEC-2026-0543` (chrono), `RUSTSEC-2026-0370`/`0371`/`0372`
(zbus_names/zvariant/zvariant_derive, transitive through accesskit/atspi). They are unmaintained
advisories, not known vulnerabilities; migration is tracked as post-0.9.4 work.

Local inventory:

```powershell
.\tests\supply-chain.ps1
```

It writes `packages.json`, `duplicates.txt` and optional `cargo-audit.txt` /
`cargo-deny.txt` under `target\supply-chain\`. If the tools are not installed, the
check is reported as `BLOCKED`; CI is the authority for the policy run.

The workspace packages declare `license = "MIT"` through `license.workspace = true`.
The release remains unsigned; hashes are published in `SHA256SUMS.txt`.
