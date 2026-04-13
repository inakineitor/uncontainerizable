import { readFile } from "node:fs/promises";

import { describe, expect, test } from "vitest";

async function readJson(relativePath: string): Promise<unknown> {
  const url = new URL(relativePath, import.meta.url);
  return JSON.parse(await readFile(url, "utf8"));
}

function readText(relativePath: string): Promise<string> {
  const url = new URL(relativePath, import.meta.url);
  return readFile(url, "utf8");
}

// Native-package invariants (napi.targets shape, optionalDependencies
// not committed, release workflow builds every target) live in the
// Rust integration test at `crates/uncontainerizable-node/tests/
// package_json_invariants.rs` because the file they check lives there
// and the checks run under `cargo test`. Only workspace-wide
// invariants belong here.

describe("workspace config regressions", () => {
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

  test("repo lint commands use the pinned ultracite binary", async () => {
    const packageJson = (await readJson("../../../package.json")) as {
      scripts: Record<string, string>;
    };
    const ciWorkflow = await readText("../../../.github/workflows/ci.yml");

    expect(packageJson.scripts.lint).toBe("pnpm exec ultracite check");
    expect(packageJson.scripts["lint:fix"]).toBe("pnpm exec ultracite fix");
    expect(ciWorkflow).toContain("run: pnpm exec ultracite check");
  });
});
