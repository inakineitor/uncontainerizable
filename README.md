# uncontainerizable

> Graceful process lifecycle for programs that can't be put in real containers.

A supervisor library for the apps you can't put in Docker: browsers, GUI
apps, and anything that needs the user's window server, keychain, or
display. Ships as a pure-Rust core plus Node bindings via
[napi-rs](https://napi.rs), so you can drive it from either runtime.

If the program can run in a real sandbox — namespaces, seccomp, landlock —
use a real container runtime. `uncontainerizable` is for everything else.

> [!NOTE]
> See [`documents/development-plan.md`](./documents/development-plan.md) for the
> authoritative design, invariants, and per-platform mechanisms.

## Features

- **Staged quit ladder.** Each platform escalates from its polite quit
  channel to a guaranteed kill, with per-stage timeouts and skippable
  stages.
- **Tree-aware teardown.** Helper processes get reaped alongside the root;
  the container is "empty" only when no member remains.
- **Identity-based singleton.** At most one container per identity is
  alive at any time; spawning preempts any predecessor using kernel
  primitives on Linux and Windows.
- **Adapter hooks.** Per-app lifecycle callbacks (`beforeQuit`,
  `beforeStage`, `afterStage`, `afterQuit`, `clearCrashState`) suppress
  "didn't shut down correctly" dialogs after force-kill.
- **Infallible destroy.** `destroy()` aggregates errors into the result
  so `finally` blocks never throw.
- **Async first.** Tokio-based core, `Promise`-based wrapper.

## How it works

| Platform          | Preemption primitive | Quit ladder                                |
| ----------------- | -------------------- | ------------------------------------------ |
| Linux (x64/arm64) | cgroup v2            | `SIGTERM` → `SIGKILL` (race-free via freeze) |
| macOS (x64/arm64) | `argv[0]` tag scan   | `aevt/quit` → `SIGTERM` → `SIGKILL`          |
| Windows (x64/arm64) | named Job Object   | `WM_CLOSE` → `TerminateJobObject`            |

Linux musl is supported via `cargo-zigbuild` cross-compilation. Identity
strings are namespaced by an app-level prefix (conventionally reverse-DNS)
so libraries using `uncontainerizable` cannot collide.

## Installation

```sh
npm install uncontainerizable
# or
pnpm add uncontainerizable
# or
yarn add uncontainerizable
```

The package depends on `@uncontainerizable/native`, which ships a
prebuilt `.node` binary per supported target. No native toolchain is
required at install time.

> [!IMPORTANT]
> Node.js ≥ 24 is required (current LTS). The TS wrapper targets
> `node24` via tsdown and uses ESM only.

## Quick start

```ts
import { App, defaultAdapters } from "uncontainerizable";

const app = new App("com.example.my-supervisor");

const container = await app.contain("chromium", {
  args: ["--user-data-dir=/tmp/browser-profile"],
  identity: "browser-main", // preempts any previous "browser-main"
  adapters: [...defaultAdapters],
});

// ... later, when you want to shut it down cleanly:
const result = await container.destroy();

if (result.errors.length > 0) {
  console.warn("teardown surfaced recoverable errors:", result.errors);
}

console.log(`exited at ${result.quit.exitedAtStage}`);
```

A second call to `app.contain(..., { identity: "browser-main" })` will
kill the running instance before launching the new one. Omit `identity`
to skip preemption entirely.

## Built-in adapters

`uncontainerizable` ships a handful of adapters for common supervised
programs. They match by probe (bundle ID, executable path, platform) and
only their `clearCrashState` hook runs after a terminal-stage teardown.

| Adapter         | Purpose                                                                 |
| --------------- | ----------------------------------------------------------------------- |
| `appkit`        | Deletes AppKit's "Saved Application State" directory on macOS.          |
| `crashReporter` | Clears per-app entries from macOS's user-level CrashReporter archive.   |
| `chromium`      | Matches Chrome/Chromium/Brave/Edge. Crash-state cleanup is stubbed.     |
| `firefox`       | Matches Firefox. Crash-state cleanup is stubbed.                        |

Import individually or as a bundle:

```ts
import {
  appkit,
  chromium,
  crashReporter,
  defaultAdapters,
  firefox,
} from "uncontainerizable";
```

## Custom adapters

An adapter is any object that matches the `Adapter` shape. Every hook
except `name` and `matches` is optional; unimplemented hooks are skipped.
Hooks may be sync or async — the wrapper normalizes both forms before
crossing the napi boundary.

```ts
import type { Adapter } from "uncontainerizable";

const logger: Adapter = {
  name: "logger",
  matches: () => true,
  beforeStage(probe, stageName) {
    console.log(`[${probe.pid}] entering stage ${stageName}`);
  },
  afterStage(_probe, result) {
    if (result.exited) {
      console.log(`drained at ${result.stageName}`);
    }
  },
};
```

> [!TIP]
> Adapter hooks are **advisory**: errors are collected into
> `QuitResult.adapterErrors` and never abort the quit ladder. A misbehaving
> adapter cannot prevent teardown.

## Rust usage

```toml
# Cargo.toml
[dependencies]
uncontainerizable-core = { git = "https://github.com/inakineitor/uncontainerizable" }
tokio = { version = "1", features = ["full"] }
```

```rust
use uncontainerizable::{App, ContainOptions, DestroyOptions};

let app = App::new("com.example.my-supervisor")?;
let mut container = app.contain("chromium", ContainOptions {
    identity: Some("browser-main".into()),
    ..Default::default()
}).await?;

let result = container.destroy(DestroyOptions::default()).await;
```

The core crate is not published to crates.io yet; depend on it from git
while the API stabilizes.

## Workspace layout

```
crates/
  uncontainerizable-core/   # Pure Rust engine (not published)
  uncontainerizable-node/   # napi-rs bindings → @uncontainerizable/native
packages/
  uncontainerizable/        # TypeScript wrapper → uncontainerizable
  tsconfig/                 # Shared TS config (workspace-private)
```

`uncontainerizable` and `@uncontainerizable/native` are linked in
Changesets and always publish at the same version. Mismatches break
`require`/`import` of the wrapper.

## Development

Prerequisites: Rust (pinned via `rust-toolchain.toml`), Node.js ≥ 24,
pnpm ≥ 10.

```sh
pnpm install              # install JS/TS dependencies
pnpm build                # full build: native bindings, then TS wrapper
pnpm build:native         # only the napi-rs build
pnpm build:ts             # only the TS wrapper
pnpm test                 # Vitest across TS packages
pnpm typecheck            # tsc --noEmit per package
pnpm lint                 # Ultracite (TS/JS/JSON)
pnpm lint:fix
pnpm lint:rust            # cargo fmt --check + cargo clippy -D warnings
cargo test --workspace --exclude uncontainerizable-node
```

> [!WARNING]
> `build:native` must run before `build:ts` on a fresh install. The TS
> wrapper's `@uncontainerizable/native` dependency resolves to the built
> `.node` loader at `crates/uncontainerizable-node/dist/`, not a stub.

Lefthook wires `rustfmt`, `clippy`, and Ultracite into `pre-commit`, and
commitlint validates Conventional Commits on `commit-msg`.

## Releasing

Changesets drives versioning. Add a changeset alongside your PR:

```sh
pnpm changeset
```

Pushing to `main` runs [`release.yml`](./.github/workflows/release.yml):

1. Builds native binaries across all target triples in parallel.
2. Assembles per-platform npm packages via `napi artifacts`.
3. Opens a "Version Packages" PR or publishes via `changesets/action`.

## License

[MIT](./LICENSE)
