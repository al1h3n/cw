# Contributing

Start with [`AGENTS.md`](AGENTS.md): rules, decisions and architecture. The work order is in
[`docs/PLAN.md`](docs/PLAN.md).

## Licence and CLA

The core is licensed under **AGPL-3.0-only** ([`LICENSE`](LICENSE)). The project is open-core (AGENTS.md,
decision D1), so outside contributions need a **Contributor Licence Agreement** before they can be
merged. The CLA text will be published once the legal entity exists. Until then, external pull
requests are welcome for discussion but will not be merged.

## Before you push

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked --no-tests=pass   # or: cargo test --workspace
cargo deny check
```

CI runs the same checks on Windows, macOS and Linux.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/): `feat(agent): …`, `fix(media): …`,
`docs: …`, `chore(ci): …`. A breaking wire-format change also bumps `proto::PROTOCOL_VERSION` and says
so in the commit body (`BREAKING CHANGE: …`).

## Spikes

`spikes/` is a separate throwaway workspace for Phase-0 experiments. It is not built by CI, and product
code never imports from it. When a spike finishes, record its numbers and the go/no-go decision in
AGENTS.md.
