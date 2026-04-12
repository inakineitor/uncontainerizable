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

export type ContainOptions = JsContainOptions;
export type QuitOptions = JsQuitOptions;
export type DestroyOptions = JsDestroyOptions;
export type QuitResult = JsQuitResult;
export type DestroyResult = JsDestroyResult;
export type StageResult = JsStageResult;
export type Probe = JsProbe;
export type Container = NodeContainer;
