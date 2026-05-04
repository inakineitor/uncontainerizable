import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout } from "node:timers/promises";
import { fileURLToPath } from "node:url";

import { App, type Container } from "uncontainerizable";
import { describe, expect, test } from "vitest";

const NESTING_TEST_PLATFORMS = new Set(["darwin", "win32"]);
const POLL_INTERVAL_MS = 50;
const READY_TIMEOUT_MS = 10_000;
const DONE_TIMEOUT_MS = 15_000;

const NESTED_CHILD_SCRIPT = `
import { readFile, writeFile } from "node:fs/promises";
import { setTimeout } from "node:timers/promises";
import { App } from "uncontainerizable";

const readyPath = process.env.UNCONTAINERIZABLE_NESTED_READY_PATH;
const releasePath = process.env.UNCONTAINERIZABLE_NESTED_RELEASE_PATH;
const donePath = process.env.UNCONTAINERIZABLE_NESTED_DONE_PATH;

if (!readyPath || !releasePath || !donePath) {
  throw new Error("missing nested test paths");
}

const app = new App(\`test.vitest.nested.child.\${process.pid}\`);
const inner = await app.contain(process.execPath, {
  args: ["--eval", "setInterval(() => undefined, 1000);"],
});
let destroyingInner = false;

async function destroyInner() {
  if (destroyingInner) {
    return;
  }
  destroyingInner = true;
  return inner.destroy();
}

process.once("SIGINT", () => {
  void destroyInner().finally(() => {
    process.exit(0);
  });
});
process.once("SIGTERM", () => {
  void destroyInner().finally(() => {
    process.exit(0);
  });
});

await writeFile(
  readyPath,
  JSON.stringify({ innerPid: await inner.pid }),
  "utf8"
);

while (true) {
  try {
    await readFile(releasePath, "utf8");
    break;
  } catch (error) {
    if (!isErrnoCode(error, "ENOENT")) {
      throw error;
    }
  }
  await setTimeout(${POLL_INTERVAL_MS});
}

const result = await destroyInner();
await writeFile(
  donePath,
  JSON.stringify({
    errors: result?.errors ?? [],
    reachedTerminalStage: result?.quit.reachedTerminalStage ?? false,
  }),
  "utf8"
);

function isErrnoCode(error, code) {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === code
  );
}
`;

type NestedReady = {
  innerPid: number;
};

type NestedDone = {
  errors: string[];
  reachedTerminalStage: boolean;
};

describe.runIf(NESTING_TEST_PLATFORMS.has(process.platform))(
  "App.contain nesting",
  () => {
    test("allows a contained process to create and destroy an inner container", async () => {
      const tempDir = await mkdtemp(
        join(tmpdir(), "uncontainerizable-nested-")
      );
      const readyPath = join(tempDir, "ready.json");
      const releasePath = join(tempDir, "release");
      const donePath = join(tempDir, "done.json");
      let outer: Container | undefined;

      try {
        const app = new App(`test.vitest.nested.parent.${process.pid}`);
        outer = await app.contain(process.execPath, {
          args: ["--input-type=module", "--eval", NESTED_CHILD_SCRIPT],
          cwd: packageRoot(),
          env: {
            UNCONTAINERIZABLE_NESTED_DONE_PATH: donePath,
            UNCONTAINERIZABLE_NESTED_READY_PATH: readyPath,
            UNCONTAINERIZABLE_NESTED_RELEASE_PATH: releasePath,
          },
        });

        const ready = await readJsonFile<NestedReady>(
          readyPath,
          READY_TIMEOUT_MS
        );
        expect(ready.innerPid).toBeGreaterThan(0);

        await writeFile(releasePath, "", "utf8");

        const done = await readJsonFile<NestedDone>(donePath, DONE_TIMEOUT_MS);
        expect(done.errors).toEqual([]);
        await waitForEmpty(outer, DONE_TIMEOUT_MS);
      } finally {
        if (outer) {
          const result = await outer.destroy();
          expect(result.errors).toEqual([]);
        }
        await rm(tempDir, { force: true, recursive: true });
      }
    }, 30_000);
  }
);

async function readJsonFile<T>(path: string, timeoutMs: number): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      return JSON.parse(await readFile(path, "utf8")) as T;
    } catch (error) {
      if (!(error instanceof SyntaxError || isErrnoCode(error, "ENOENT"))) {
        throw error;
      }
    }
    await setTimeout(POLL_INTERVAL_MS);
  }
  throw new Error(`timed out waiting for ${path}`);
}

async function waitForEmpty(
  container: Container,
  timeoutMs: number
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await container.isEmpty()) {
      return;
    }
    await setTimeout(POLL_INTERVAL_MS);
  }
  throw new Error("timed out waiting for outer container to exit");
}

function isErrnoCode(error: unknown, code: string): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    (error as { code?: unknown }).code === code
  );
}

function packageRoot(): string {
  return fileURLToPath(new URL("../", import.meta.url));
}
