# Contributing

Use a branch and pull request for changes. CI checks formatting, Clippy, tests,
and packaging on Linux and Windows. Keep protocol fixtures synthetic: never
commit household inventories, addresses, footage, credentials, or raw API dumps.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo package --locked
```

New adapters should have explicit timeouts, bounded responses, typed results,
and tests for malformed replies and partial failure. Keep observations separate
from interpretation. A successful API response does not establish physical state.

Prefer small upstreamable changes to upstream forks, and focused adapters here.
The initial package is not published to crates.io. Releases and a stable API are
future milestones; install the reviewed revision from Git in the meantime.
