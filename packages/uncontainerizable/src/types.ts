import type {
  JsContainOptions,
  JsDestroyOptions,
  JsDestroyResult,
  JsProbe,
  JsQuitOptions,
  JsQuitResult,
  JsStageResult,
  NodeContainer,
} from "@uncontainerizable/native";

export type SupportedPlatform = "linux" | "darwin" | "win32";

export type QuitOptions = JsQuitOptions;
export type DestroyOptions = JsDestroyOptions;
export type QuitResult = JsQuitResult;
export type DestroyResult = JsDestroyResult;
export type StageResult = JsStageResult;
export type Probe = JsProbe;

/**
 * Platform-agnostic handle for a spawned contained process. Returned by
 * `App.contain`; backed by the napi-generated `NodeContainer` class.
 */
export type Container = NodeContainer;

/**
 * Per-app lifecycle hook. Each method except `name` and `matches` is
 * optional; unimplemented hooks are skipped by the Rust orchestrator.
 *
 * Every hook may return a value or a Promise; the TypeScript wrapper
 * normalizes both forms to `Promise<_>` before handing the adapter to
 * the napi bridge. Hooks must not throw synchronously: a thrown value
 * is trapped by the bridge and recorded in
 * `QuitResult.adapterErrors`, but ergonomic code should prefer
 * returning a rejected Promise.
 */
export type Adapter = {
  readonly name: string;
  matches(probe: Probe): boolean | Promise<boolean>;
  beforeQuit?(probe: Probe): void | Promise<void>;
  beforeStage?(probe: Probe, stageName: string): void | Promise<void>;
  afterStage?(probe: Probe, result: StageResult): void | Promise<void>;
  afterQuit?(probe: Probe, result: QuitResult): void | Promise<void>;
  clearCrashState?(probe: Probe): void | Promise<void>;
};

/**
 * Extends the napi-generated options with a TypeScript-only `adapters`
 * field. The wrapper normalizes each adapter's hook methods to the
 * async signatures the napi bridge expects before crossing the
 * boundary.
 */
export type ContainOptions = Omit<JsContainOptions, "adapters"> & {
  adapters?: Adapter[];
};
