import type {
  JsContainOptions,
  JsDestroyOptions,
  JsDestroyResult,
  JsProbe,
  JsQuitOptions,
  JsQuitResult,
  JsStageResult,
} from "@uncontainerizable/native";

export type SupportedPlatform = "linux" | "darwin" | "win32";

export type QuitOptions = JsQuitOptions;
export type DestroyOptions = JsDestroyOptions;
export type QuitResult = JsQuitResult;
export type DestroyResult = JsDestroyResult;
export type StageResult = JsStageResult;
export type Probe = JsProbe;

/**
 * Platform-agnostic handle for a spawned contained process.
 *
 * The concrete instance returned by `App.contain` is a TypeScript wrapper
 * that interposes adapter hooks on top of the native container returned
 * by the napi bridge. Consumers interact with it only through this type.
 */
export type Container = {
  readonly pid: Promise<number>;
  readonly probe: Promise<Probe>;
  members(): Promise<number[]>;
  isEmpty(): Promise<boolean>;
  quit(opts?: QuitOptions): Promise<QuitResult>;
  destroy(opts?: DestroyOptions): Promise<DestroyResult>;
};

/**
 * Per-app lifecycle hook. Every method except `name` and `matches` is
 * optional; unimplemented hooks are skipped.
 *
 * v0.1 only invokes `clearCrashState` (after `destroy` reaches the
 * terminal stage). The other hooks are declared for forward compatibility
 * so an adapter written today does not need changes when the wrapper
 * grows per-stage invocation.
 */
export type Adapter = {
  readonly name: string;
  matches(probe: Probe): boolean | Promise<boolean>;
  beforeQuit?(probe: Probe, container: Container): Promise<void> | void;
  beforeStage?(
    probe: Probe,
    stageName: string,
    container: Container
  ): Promise<void> | void;
  afterStage?(
    probe: Probe,
    result: StageResult,
    container: Container
  ): Promise<void> | void;
  afterQuit?(
    probe: Probe,
    result: QuitResult,
    container: Container
  ): Promise<void> | void;
  clearCrashState?(probe: Probe): Promise<void> | void;
};

/**
 * Extends the napi-generated options with a TypeScript-only `adapters`
 * field. Adapters run on the JS side around the native destroy call.
 */
export type ContainOptions = JsContainOptions & {
  adapters?: Adapter[];
};
