import { mkdir, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  type Probe,
  appkit,
  chromium,
  defaultAdapters,
  firefox,
} from "uncontainerizable";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

function darwinProbe(overrides: Partial<Probe> = {}): Probe {
  return {
    pid: 123,
    bundleId: null,
    executablePath: null,
    platform: "darwin",
    capturedAtMs: 0,
    ...overrides,
  };
}

function linuxProbe(overrides: Partial<Probe> = {}): Probe {
  return {
    pid: 123,
    bundleId: null,
    executablePath: null,
    platform: "linux",
    capturedAtMs: 0,
    ...overrides,
  };
}

describe("appkit.matches", () => {
  test("matches a darwin probe with a bundle id", () => {
    expect(
      appkit.matches(darwinProbe({ bundleId: "com.apple.TextEdit" }))
    ).toBe(true);
  });

  test("does not match when the bundle id is missing", () => {
    expect(appkit.matches(darwinProbe({ bundleId: null }))).toBe(false);
  });

  test("does not match a non-darwin probe", () => {
    expect(linuxProbe({ bundleId: "com.apple.TextEdit" })).toBeTruthy();
    expect(appkit.matches(linuxProbe({ bundleId: "com.apple.TextEdit" }))).toBe(
      false
    );
  });
});

describe("chromium.matches", () => {
  test("matches canonical chrome basenames", () => {
    for (const exe of [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/usr/bin/chromium",
      "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ]) {
      expect(
        chromium.matches(darwinProbe({ executablePath: exe })),
        `expected match for ${exe}`
      ).toBe(true);
    }
  });

  test("does not match firefox", () => {
    expect(
      chromium.matches(darwinProbe({ executablePath: "/usr/bin/firefox" }))
    ).toBe(false);
  });

  test("does not match when executable path is missing", () => {
    expect(chromium.matches(darwinProbe({ executablePath: null }))).toBe(false);
  });
});

describe("firefox.matches", () => {
  test("matches firefox basenames", () => {
    for (const exe of [
      "/usr/bin/firefox",
      "/Applications/Firefox.app/Contents/MacOS/firefox",
      "/usr/lib/firefox/firefox-bin",
    ]) {
      expect(
        firefox.matches(darwinProbe({ executablePath: exe })),
        `expected match for ${exe}`
      ).toBe(true);
    }
  });

  test("does not match chrome", () => {
    expect(
      firefox.matches(darwinProbe({ executablePath: "/usr/bin/chrome" }))
    ).toBe(false);
  });
});

describe("defaultAdapters", () => {
  test("bundles all three built-ins", () => {
    expect(defaultAdapters.map((a) => a.name)).toEqual([
      "chromium",
      "firefox",
      "appkit",
    ]);
  });
});

describe("appkit.clearCrashState", () => {
  let testHome: string;
  let savedStateDir: string;
  let originalHome: string | undefined;

  beforeEach(async () => {
    testHome = await createTempHome();
    originalHome = process.env.HOME;
    process.env.HOME = testHome;
    savedStateDir = join(
      testHome,
      "Library",
      "Saved Application State",
      "com.example.uncont-test.savedState"
    );
    await mkdir(savedStateDir, { recursive: true });
    await writeFile(join(savedStateDir, "windows.plist"), "dummy state");
  });

  afterEach(() => {
    if (originalHome === undefined) {
      delete process.env.HOME;
    } else {
      process.env.HOME = originalHome;
    }
  });

  test("removes the saved-state directory when it exists", async () => {
    await expect(stat(savedStateDir)).resolves.toBeDefined();
    await appkit.clearCrashState?.(
      darwinProbe({ bundleId: "com.example.uncont-test" })
    );
    await expect(stat(savedStateDir)).rejects.toMatchObject({ code: "ENOENT" });
  });

  test("is a no-op when the saved-state directory does not exist", async () => {
    const probe = darwinProbe({ bundleId: "com.example.uncont-no-state" });
    await expect(appkit.clearCrashState?.(probe)).resolves.toBeUndefined();
  });

  test("does nothing when the bundle id is absent", async () => {
    const probe = darwinProbe({ bundleId: null });
    await appkit.clearCrashState?.(probe);
    await expect(stat(savedStateDir)).resolves.toBeDefined();
  });
});

describe("chromium + firefox stub warnings", () => {
  test("chromium.clearCrashState warns and returns", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    try {
      chromium.clearCrashState?.(darwinProbe());
      expect(warn).toHaveBeenCalledWith(
        expect.stringContaining("chromium.clearCrashState is a v0.1 stub")
      );
    } finally {
      warn.mockRestore();
    }
  });

  test("firefox.clearCrashState warns and returns", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    try {
      firefox.clearCrashState?.(darwinProbe());
      expect(warn).toHaveBeenCalledWith(
        expect.stringContaining("firefox.clearCrashState is a v0.1 stub")
      );
    } finally {
      warn.mockRestore();
    }
  });
});

async function createTempHome(): Promise<string> {
  const dir = join(tmpdir(), `uncont-test-${process.pid}-${Date.now()}`);
  await mkdir(dir, { recursive: true });
  return dir;
}
