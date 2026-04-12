import { type JsAdapter, NodeApp } from "@uncontainerizable/native";

import type { Adapter, ContainOptions, Container } from "#/types.js";

export { coreVersion } from "@uncontainerizable/native";

export {
  appkit,
  chromium,
  crashReporter,
  defaultAdapters,
  firefox,
} from "#/adapters/index.js";

/**
 * Namespaced handle for spawning contained processes.
 *
 * Construct once per application, typically with a reverse-DNS string like
 * `"com.example.my-supervisor"`. The prefix namespaces identity strings so
 * two unrelated libraries using `uncontainerizable` cannot collide.
 *
 * Throws `INVALID_IDENTITY` if the prefix contains characters outside
 * `[A-Za-z0-9._:-]`.
 */
export class App {
  readonly #inner: NodeApp;

  constructor(prefix: string) {
    this.#inner = new NodeApp(prefix);
  }

  get prefix(): string {
    return this.#inner.prefix;
  }

  /**
   * Spawn a contained process. If `options.identity` is set, any previous
   * instance with the same (prefix, identity) pair is killed before this
   * one launches. If `options.adapters` is non-empty the Rust
   * orchestrator drives their lifecycle hooks around the quit ladder.
   */
  contain(command: string, options: ContainOptions = {}): Promise<Container> {
    const { adapters, ...nativeOpts } = options;
    return this.#inner.contain(command, {
      ...nativeOpts,
      adapters: adapters?.map(normalizeAdapter),
    });
  }
}

/**
 * Normalize a user-provided `Adapter` (which may declare sync methods)
 * into the always-async shape the napi bridge expects. Every optional
 * hook is only forwarded if defined, so the Rust side keeps its
 * "undefined means skip" semantics.
 */
function normalizeAdapter(adapter: Adapter): JsAdapter {
  return {
    name: adapter.name,
    matches: async (probe) => Boolean(await adapter.matches(probe)),
    beforeQuit: adapter.beforeQuit
      ? async (probe) => {
          await adapter.beforeQuit?.(probe);
        }
      : undefined,
    beforeStage: adapter.beforeStage
      ? async (probe, stageName) => {
          await adapter.beforeStage?.(probe, stageName);
        }
      : undefined,
    afterStage: adapter.afterStage
      ? async (probe, result) => {
          await adapter.afterStage?.(probe, result);
        }
      : undefined,
    afterQuit: adapter.afterQuit
      ? async (probe, result) => {
          await adapter.afterQuit?.(probe, result);
        }
      : undefined,
    clearCrashState: adapter.clearCrashState
      ? async (probe) => {
          await adapter.clearCrashState?.(probe);
        }
      : undefined,
  };
}

export type {
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
} from "#/types.js";
