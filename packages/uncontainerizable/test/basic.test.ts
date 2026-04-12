import { App, coreVersion } from "uncontainerizable";
import { describe, expect, test } from "vitest";

const VERSION_PATTERN = /^\d+\.\d+\.\d+/;
const INVALID_IDENTITY_PATTERN = /INVALID_IDENTITY/;
const UNSUPPORTED_PLATFORM_PATTERN = /UNSUPPORTED_PLATFORM/;
const SUPPORTED_PLATFORMS = new Set(["darwin", "linux", "win32"]);

describe("uncontainerizable", () => {
  test("coreVersion returns the Rust crate version string", () => {
    expect(coreVersion()).toMatch(VERSION_PATTERN);
  });

  test("App rejects an empty prefix", () => {
    expect(() => new App("")).toThrowError(INVALID_IDENTITY_PATTERN);
  });

  test("App rejects a prefix with whitespace", () => {
    expect(() => new App("has space")).toThrowError(INVALID_IDENTITY_PATTERN);
  });

  test("App rejects a prefix with a slash", () => {
    expect(() => new App("has/slash")).toThrowError(INVALID_IDENTITY_PATTERN);
  });

  test("App exposes the normalized prefix", () => {
    const app = new App("com.example.test");
    expect(app.prefix).toBe("com.example.test");
  });
});

describe.runIf(process.platform === "darwin")("App.contain on Darwin", () => {
  test("spawns sleep and destroys cleanly at sigterm_tree", async () => {
    const app = new App("test.vitest.basic");
    const container = await app.contain("sleep", {
      args: ["30"],
      darwinTagArgv0: false,
    });
    const result = await container.destroy();
    expect(result.errors).toEqual([]);
    // `sleep` reacts to SIGTERM, so destroy should not escalate to
    // sigkill_tree. The exit stage may be `sigterm_tree` or
    // `before:<next>` depending on polling timing.
    expect(result.quit.reachedTerminalStage).toBe(false);
    expect(result.quit.stageResults.length).toBeGreaterThanOrEqual(1);
  }, 20_000);
});

describe.runIf(!SUPPORTED_PLATFORMS.has(process.platform))(
  "App.contain on unsupported platforms",
  () => {
    test("throws UNSUPPORTED_PLATFORM", async () => {
      const app = new App("test.vitest.unsupported");
      await expect(app.contain("sleep", { args: ["5"] })).rejects.toThrow(
        UNSUPPORTED_PLATFORM_PATTERN
      );
    });
  }
);
