# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`uncontainerizable` is a supervisor library for programs that can't be put in real containers (browsers, GUI apps, anything needing the user's window server/keychain/display). Pure-Rust core + napi-rs Node bindings, published as `uncontainerizable` (TS wrapper) and `@uncontainerizable/native` (bindings).

Design reference: **`documents/development-plan.md`**. It is authoritative for concepts, invariants, per-platform mechanisms, and the expected module layout. Read it before making nontrivial changes to the core or bindings.

## Workspace topology

Three publishable units and one private tsconfig package:

- `crates/uncontainerizable-core/`: pure Rust lib (the engine; not published).
- `crates/uncontainerizable-node/`: `cdylib` with napi-rs bindings. **This crate is simultaneously a pnpm workspace member** (see `pnpm-workspace.yaml`). Its `package.json` ships as `@uncontainerizable/native`.
- `packages/uncontainerizable/`: TS wrapper that re-exports from `@uncontainerizable/native`. Depends on it via `workspace:*`.
- `packages/tsconfig/`: shared `@uncontainerizable/tsconfig/base.json`, workspace-private.

The two publishable packages are **linked in Changesets** (`.changeset/config.json`). They must always ship at the same version. Mismatches cause runtime `require` failures for consumers.

## Build & task commands

```sh
pnpm install              # pnpm 10.x, see packageManager in package.json
pnpm build                # Full build via turbo; native first, then TS, in topological order
pnpm build:native         # Only the napi-rs build
pnpm build:ts             # Only tsdown; depends on ^build (upstream native)
pnpm test                 # Vitest across TS packages
pnpm typecheck            # tsc --noEmit per TS package
pnpm lint                 # Ultracite check (TS/JS/JSON)
pnpm lint:fix             # Ultracite fix
pnpm lint:rust            # cargo fmt --check + cargo clippy -D warnings
pnpm lint:package         # publint against every publishable package

cargo check --workspace   # Rust-only compile check
cargo test --workspace --exclude uncontainerizable-node
pnpm --filter uncontainerizable test -- <pattern>   # single vitest file/name
```

**`build:native` must run before `build:ts` in the same pnpm install.** The TS wrapper is not a stub; its `@uncontainerizable/native` dep resolves to a built `.node` + loader at `crates/uncontainerizable-node/dist/`. If you've modified the Rust surface, rebuild native before running typecheck or vitest.

## napi-rs output layout (non-default)

The native crate builds into `crates/uncontainerizable-node/dist/`, not the crate root:

```
napi build --platform --release --output-dir dist --js index.js --dts index.d.ts
```

Output is `dist/index.js` (loader), `dist/index.d.ts`, `dist/<binaryName>.<platform>.node`. `package.json` `main`/`types`/`files` point at `./dist/...`. If you change this, update `turbo.json` outputs and the release workflow's `upload-artifact` path together, since both reference this directory explicitly.

## Platform dispatch (core)

Per the dev plan, `platforms::spawn(app, command, opts)` is the single entry point into platform code. It `cfg`-dispatches to `linux`, `darwin`, or `win32`. Each platform owns its own `stages`, `probe`, and preemption primitive:

- **Linux**: cgroup v2 at `{session}/uncontainerizable/{identity}/`. Identity preemption uses `mkdir` atomicity; teardown uses `cgroup.freeze` for race-free SIGKILL.
- **Windows**: Named Job Object `Local\uncontainerizable-{identity}`. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` guarantees cleanup if the supervisor dies.
- **macOS**: Best-effort via `argv[0]` tagging (`uncontainerizable:{identity}/original-name`). Caller can opt out via `ContainOptions.darwin_tag_argv0 = false`.

Adapter hooks are **advisory**: errors are collected into the final result, never abort escalation. `destroy()` is infallible (never throws; aggregates errors). See `documents/development-plan.md` "Invariants" for the full list.

## TypeScript conventions (TS 6.0)

- Source lives in `packages/uncontainerizable/src/`. Internal imports use the **subpath import** pattern: `import type { X } from "#/types.js"` resolves to `./src/types.ts` via `"imports": { "#/*": "./src/*" }` in the wrapper's `package.json`.
- Base tsconfig is intentionally opinionated for TS 6:
  - `target`/`lib`: `ESNext`. The wrapper uses `noEmit: true`, so tsc's `target` doesn't control emit (tsdown/rolldown does, with its own `target: "node24"` in `tsdown.config.ts`). Runtime API availability is fenced by `@types/node` + `engines.node` (`>=24`, current LTS) + CI, not by a narrower `lib`.
  - `types: []` in base; packages must declare `types: ["node"]` (TS 6 no longer auto-loads all `@types/*`).
  - `moduleResolution: "NodeNext"`, `moduleDetection: "force"`, `verbatimModuleSyntax: true`.
- Wrapper ships dual CJS/ESM via tsdown; `tsdown.config.ts` externalizes `@uncontainerizable/native` so the bundler doesn't try to inline the loader.

## Quality gates

Biome (via Ultracite preset) handles TS/JS/JSON/CSS formatting and linting. **Don't add ESLint or Prettier.** Root config extends `ultracite/biome/core` + `ultracite/biome/vitest`. Two Ultracite rules are disabled repo-wide because they fight library ergonomics: `performance/noBarrelFile` and `performance/noReExportAll` (the wrapper's entry points re-export by design).

Rust: `rustfmt` + `clippy` with `-D warnings`. Lefthook runs both plus Ultracite on `pre-commit`, and commitlint (Conventional Commits) on `commit-msg`. Commits must be conventional.

`publint` runs as `lint:package` per publishable package and in CI; its warnings fail the build. Treat them as errors, not suggestions.

## CI / release

- `.github/workflows/ci.yml` on PR + push to `main`: lint (Ultracite + cargo fmt/clippy + publint), rust test matrix (ubuntu/macos/windows), node test matrix (same, with a native build step per runner).
- `.github/workflows/release.yml` on push to `main`: 8-target `build-native` matrix (macOS x64/arm64, Linux gnu/musl x64/arm64 via `cargo-zigbuild`+zig for musl, Windows x64/arm64). `napi artifacts` assembles `npm/` per-platform packages; `changesets/action` opens a Version Packages PR or publishes. `NPM_CONFIG_PROVENANCE=true` is set.

## Things that will bite you

- Changing the `napi` field in `crates/uncontainerizable-node/package.json` (`binaryName` or `targets`) must be mirrored in the release workflow matrix. They are not auto-generated from each other.
- The `.node` binary is platform-specific. A local `pnpm build` only produces the host target; cross-target binaries only exist after CI.
- Ultracite v7's `extends` uses submodule paths (`ultracite/biome/core`), not the v6 `"ultracite"` shorthand. Don't "fix" this.
- `Cargo.lock` is gitignored (library workspace convention). CI regenerates; commit it if you need fully reproducible Rust builds.
