# uncontainerizable

Graceful process lifecycle for programs that cannot be put in real containers:
browsers, GUI apps, and tools that need the user's window server, keychain, or
display. Rust core with async APIs plus Node bindings via napi-rs.

See [`documents/development-plan.md`](./documents/development-plan.md) for the
full design and rationale.

## Repository layout

```
crates/uncontainerizable-core/   # Pure Rust library (the engine)
crates/uncontainerizable-node/   # napi-rs bindings (published as @uncontainerizable/native)
packages/uncontainerizable/      # TypeScript wrapper (published as uncontainerizable)
packages/tsconfig/               # Shared TS config (workspace-private)
```

## Platform support

| Platform          | Preemption mechanism | Stages                            |
| ----------------- | -------------------- | --------------------------------- |
| Linux (x64/arm)   | cgroup v2            | SIGTERM then SIGKILL (cgroup freeze) |
| macOS (x64/arm)   | argv[0] tag scanning | aevt/quit then SIGTERM then SIGKILL  |
| Windows (x64/arm) | named Job Object     | WM_CLOSE then TerminateJobObject     |

Linux musl is supported via `cargo-zigbuild` cross-compilation.

## Prerequisites

- Rust (pinned via `rust-toolchain.toml` to the current stable channel with
  `rustfmt` and `clippy`)
- Node.js ≥ 24 (current LTS)
- pnpm ≥ 10 (version declared in `packageManager`)

## Build

```sh
pnpm install          # Install JS/TS dependencies
pnpm build            # Full build: native bindings first, then TS wrapper
pnpm build:native     # Just the napi-rs native bindings
pnpm build:ts         # Just the TS wrapper (requires native built first)
```

## Test

```sh
pnpm test             # Vitest across TS packages
cargo test --workspace --exclude uncontainerizable-node
```

The `uncontainerizable-node` crate is a `cdylib`, which `cargo test` can't
execute directly. Its behavior is verified through the Vitest suite against
the built `.node` binary.

## Lint and format

Biome (via Ultracite) handles TS/JS/JSON/CSS. Rust uses `rustfmt` and
`clippy`. Lefthook wires both into `pre-commit`; `commit-msg` validates
conventional commits via commitlint.

```sh
pnpm lint             # Ultracite check (TS/JS/JSON)
pnpm lint:fix         # Ultracite fix
pnpm lint:rust        # cargo fmt --check + cargo clippy -D warnings
```

## TypeScript conventions

The TS wrapper is configured for TypeScript 6+ and uses the
[subpath imports](https://devblogs.microsoft.com/typescript/announcing-typescript-6-0/)
pattern introduced in TS 6.0:

- `#/*` maps to `./src/*` via the `imports` field in `package.json`, so internal
  modules use `import { X } from "#/types.js"` rather than brittle relative
  paths.
- `moduleResolution: "NodeNext"` (TS 6.0 deprecates the legacy `"node"` mode).
- `types: ["node"]` is declared explicitly per TS 6.0's new default of `[]`.
- `target` and `lib` set to `ESNext`; runtime API surface is fenced by
  `@types/node` + `engines.node` (`>=24`) + CI rather than a narrower `lib`.
  tsdown (rolldown) handles the actual emit and has its own `target: "node24"`.
- `verbatimModuleSyntax: true` and `moduleDetection: "force"` for clean ESM
  emit; `esModuleInterop: true` (TS 6.0 no longer allows `false`).

## Release

Changesets drives versioning. Add a changeset alongside your PR:

```sh
pnpm changeset
```

`uncontainerizable` and `@uncontainerizable/native` are `linked` so they always
publish at the same version. Mismatches break `require`/`import` of the wrapper.

Pushing to `main` triggers `.github/workflows/release.yml`:

1. Build native binaries across all target triples in parallel.
2. Upload `.node` artifacts per target.
3. `napi artifacts` assembles the per-platform npm packages.
4. `changesets/action` either opens a "Version Packages" PR or publishes.

## License

MIT. See [`LICENSE`](./LICENSE).
