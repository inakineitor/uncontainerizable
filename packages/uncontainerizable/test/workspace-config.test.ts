import { readFile } from "node:fs/promises";

import { describe, expect, test } from "vitest";

async function readJson(relativePath: string): Promise<unknown> {
  const url = new URL(relativePath, import.meta.url);
  return JSON.parse(await readFile(url, "utf8"));
}

describe("workspace config regressions", () => {
  test("native optional dependency versions stay aligned with the package version", async () => {
    const nativePackage = (await readJson(
      "../../../crates/uncontainerizable-node/package.json"
    )) as {
      optionalDependencies: Record<string, string>;
      version: string;
    };

    for (const version of Object.values(nativePackage.optionalDependencies)) {
      expect(version).toBe(nativePackage.version);
    }
  });

  test("turbo runs the current package build before tests", async () => {
    const turbo = (await readJson("../../../turbo.json")) as {
      tasks: {
        test?: {
          dependsOn?: string[];
        };
      };
    };

    expect(turbo.tasks.test?.dependsOn).toContain("build");
  });
});
