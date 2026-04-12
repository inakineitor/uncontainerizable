import { NodeApp, type NodeContainer } from "@uncontainerizable/native";

import type {
  Adapter,
  ContainOptions,
  Container,
  DestroyOptions,
  DestroyResult,
  Probe,
  QuitOptions,
  QuitResult,
} from "#/types.js";

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
   * one launches. If `options.adapters` is non-empty, matching adapters
   * get their `clearCrashState` hook invoked after `destroy` reaches the
   * terminal stage.
   */
  async contain(
    command: string,
    options: ContainOptions = {}
  ): Promise<Container> {
    const { adapters, ...nativeOpts } = options;
    const inner = await this.#inner.contain(command, nativeOpts);
    return new ContainerWithAdapters(inner, adapters ?? []);
  }
}

/**
 * TypeScript-side proxy that forwards to `NodeContainer` and invokes
 * matching adapters' `clearCrashState` hook after a terminal-stage
 * destroy. Every hook error is swallowed into the returned
 * `DestroyResult.quit.adapterErrors`; adapters never abort teardown.
 */
class ContainerWithAdapters implements Container {
  readonly #inner: NodeContainer;
  readonly #adapters: readonly Adapter[];

  constructor(inner: NodeContainer, adapters: readonly Adapter[]) {
    this.#inner = inner;
    this.#adapters = adapters;
  }

  get pid(): Promise<number> {
    return Promise.resolve(this.#inner.pid);
  }

  get probe(): Promise<Probe> {
    return Promise.resolve(this.#inner.probe);
  }

  members(): Promise<number[]> {
    return this.#inner.members();
  }

  isEmpty(): Promise<boolean> {
    return this.#inner.isEmpty();
  }

  quit(opts?: QuitOptions): Promise<QuitResult> {
    return this.#inner.quit(opts);
  }

  async destroy(opts?: DestroyOptions): Promise<DestroyResult> {
    const probe = await this.#inner.probe;
    const result = await this.#inner.destroy(opts);
    if (result.quit.reachedTerminalStage && this.#adapters.length > 0) {
      await this.#runClearCrashState(probe, result);
    }
    return result;
  }

  async #runClearCrashState(
    probe: Probe,
    result: DestroyResult
  ): Promise<void> {
    for (const adapter of this.#adapters) {
      let matched: boolean;
      try {
        matched = await adapter.matches(probe);
      } catch (err) {
        result.quit.adapterErrors.push(
          `${adapter.name}: matches() failed: ${errorMessage(err)}`
        );
        continue;
      }
      if (!(matched && adapter.clearCrashState)) {
        continue;
      }
      try {
        await adapter.clearCrashState(probe);
      } catch (err) {
        result.quit.adapterErrors.push(
          `${adapter.name}: clearCrashState() failed: ${errorMessage(err)}`
        );
      }
    }
  }
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }
  return String(err);
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
