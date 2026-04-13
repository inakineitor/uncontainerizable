# uncontainerizable

> Graceful process lifecycle for programs that can't be put in real containers.

A supervisor for the apps you can't put in Docker: browsers, GUI apps,
and anything that needs the user's window server, keychain, or display.
Built on a pure-Rust core with Node bindings via
[napi-rs](https://napi.rs); the published binary is prebuilt for every
supported target.

If the program can run in a real sandbox — namespaces, seccomp, landlock
— use a real container runtime. `uncontainerizable` is for everything else.

## Features

- **Staged quit ladder.** Each platform escalates from its polite quit
  channel to a guaranteed kill, with per-stage timeouts and skippable
  stages.
- **Tree-aware teardown.** Helper processes get reaped alongside the root;
  the container is "empty" only when no member remains.
- **Identity-based preemption.** Spawning with an `identity` preempts
  earlier matching instances. Linux and Windows are identity-scoped;
  macOS `.app` launches use bundle-scoped preemption on the Launch
  Services path.
- **Adapter hooks.** Per-app lifecycle callbacks suppress "didn't shut
  down correctly" dialogs after force-kill.
- **Infallible destroy.** `destroy()` aggregates errors into the result
  so `finally` blocks never throw.
- **Async first.** `Promise`-based API, Tokio-backed core.

## Platform support

| Platform          | Preemption primitive | Quit ladder                                |
| ----------------- | -------------------- | ------------------------------------------ |
| Linux (x64/arm64) | cgroup v2            | `SIGTERM` → `SIGKILL` (race-free via freeze) |
| macOS (x64/arm64) | `argv[0]` tag scan / bundle-exec `ps` scan | `aevt/quit` → `SIGTERM` → `SIGKILL` |
| Windows (x64/arm64) | named Job Object   | `WM_CLOSE` → `TerminateJobObject`            |

Linux musl is shipped via `cargo-zigbuild`. Identity strings are
namespaced by an app-level prefix (conventionally reverse-DNS) so
libraries using `uncontainerizable` cannot collide.

On macOS, direct-exec launches use `argv[0]` tag scanning. Launch
Services `.app` launches instead match by bundle executable path via
`ps comm=`, so supplying `identity` there kills any running instance of
that bundle before relaunch, regardless of which identity started it.

## Installation

```sh
npm install uncontainerizable
# or
pnpm add uncontainerizable
# or
yarn add uncontainerizable
```

The package pulls in `@uncontainerizable/native`, which resolves to a
prebuilt `.node` binary for the current platform. No native toolchain
is needed at install time.

> [!IMPORTANT]
> Node.js ≥ 24 is required (current LTS). The package ships ESM only.

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
kill the running instance before launching the new one. On macOS `.app`
launches, the same option acts as a bundle-scoped clean-slate switch and
clears any running instance of that bundle. Omit `identity` to skip
preemption entirely.

## API

### `new App(prefix)`

Namespaced handle for spawning contained processes. The `prefix`
(conventionally reverse-DNS, e.g. `"com.example.my-supervisor"`)
namespaces identity strings so unrelated libraries can't collide.

Throws `INVALID_IDENTITY` if `prefix` contains characters outside
`[A-Za-z0-9._:-]`.

### `app.contain(command, options?) → Promise<Container>`

Spawns a contained process. Options:

| Field             | Type             | Notes                                                            |
| ----------------- | ---------------- | ---------------------------------------------------------------- |
| `args`            | `string[]`       | Command-line arguments.                                          |
| `env`             | `Record<…>`      | Environment overrides.                                           |
| `cwd`             | `string`         | Working directory.                                               |
| `identity`        | `string`         | Enables preemption; macOS `.app` launches match by bundle, other routes by identity. |
| `adapters`        | `Adapter[]`      | Per-app lifecycle hooks.                                         |
| `darwinTagArgv0`  | `boolean`        | macOS direct-exec only; set `false` if the managed program misreads argv[0] (ignored for `.app` bundle launches). |

### `container.quit(options?) → Promise<QuitResult>`

Runs the staged quit ladder without releasing platform resources. Use
when you want to wait for the process to drain but keep the container
handle alive.

### `container.destroy(options?) → Promise<DestroyResult>`

Runs the quit ladder and releases platform resources. Always resolves:
recoverable errors appear in `result.errors`, never as a thrown
exception.

### `coreVersion() → string`

Returns the Rust core's version string — handy for logging and support.

## Built-in adapters

Adapters match by probe (bundle ID, executable path, platform). Their
`clearCrashState` hook runs after a terminal-stage teardown to suppress
restart dialogs.

| Adapter         | Purpose                                                              |
| --------------- | -------------------------------------------------------------------- |
| `appkit`        | Deletes AppKit's "Saved Application State" directory on macOS.       |
| `crashReporter` | Clears per-app entries from macOS's user-level CrashReporter archive.|
| `chromium`      | Matches Chrome/Chromium/Brave/Edge. Crash-state cleanup is stubbed.  |
| `firefox`       | Matches Firefox. Crash-state cleanup is stubbed.                     |

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

An adapter is any object matching the `Adapter` shape. Every hook except
`name` and `matches` is optional; unimplemented hooks are skipped. Hooks
may be sync or async — the wrapper normalizes both forms before crossing
the napi boundary.

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

Hook surface:

- `beforeQuit(probe)` — before the ladder starts.
- `beforeStage(probe, stageName)` / `afterStage(probe, result)` — around
  each stage.
- `afterQuit(probe, result)` — after the ladder ends, terminal or not.
- `clearCrashState(probe)` — only after a terminal-stage teardown.

> [!TIP]
> Adapter hooks are **advisory**: errors are collected into
> `QuitResult.adapterErrors` and never abort the quit ladder. A
> misbehaving adapter cannot prevent teardown.

## TypeScript

All public types are exported from the package root:

```ts
import type {
  Adapter,
  ContainOptions,
  Container,
  DestroyOptions,
  DestroyResult,
  Probe,
  QuitOptions,
  QuitResult,
  StageResult,
  SupportedPlatform,
} from "uncontainerizable";
```

## License

[MIT](https://github.com/inakineitor/uncontainerizable/blob/main/LICENSE)
