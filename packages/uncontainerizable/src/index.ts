import { NodeApp } from "@uncontainerizable/native";

import type { ContainOptions, Container } from "#/types.js";

export { coreVersion } from "@uncontainerizable/native";

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
   * one launches.
   */
  contain(command: string, options: ContainOptions = {}): Promise<Container> {
    return this.#inner.contain(command, options);
  }
}

export type {
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
