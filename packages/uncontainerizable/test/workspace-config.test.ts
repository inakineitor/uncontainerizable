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

describe("workspace config regressions", () => {
  test("native package does not commit its own optional dependencies", async () => {
    // `napi prepublish` regenerates the native package's
    // optionalDependencies from `napi.targets` immediately before
    // publish. Keeping the field in source would drift against that
    // generated state forever: bumping the package version in a
    // Changesets PR would need a matching bump of eight pinned
    // sibling packages that don't yet exist on npm, producing a
    // lockfile pnpm can't resolve. Source should stay free of
    // self-referential @uncontainerizable/native-* pins so that
    // pitfall can't come back.
    const nativePackage = (await readJson(
      "../../../crates/uncontainerizable-node/package.json"
    )) as {
      dependencies?: Record<string, string>;
      devDependencies?: Record<string, string>;
      optionalDependencies?: Record<string, string>;
      peerDependencies?: Record<string, string>;
    };

    const depFields = [
      "dependencies",
      "devDependencies",
      "optionalDependencies",
      "peerDependencies",
    ] as const;
    for (const field of depFields) {
      const entries = nativePackage[field] ?? {};
      for (const name of Object.keys(entries)) {
        expect(
          name.startsWith("@uncontainerizable/native-"),
          `${field} must not list @uncontainerizable/native-* package ${name}; napi prepublish generates optionalDependencies at publish time`
        ).toBe(false);
      }
    }
  });

  test("release workflow matrix matches napi.targets", async () => {
    // `napi.targets` is the single source of truth for per-platform
    // binaries. `napi prepublish` reads it at publish time to generate
    // optionalDependencies for the main package (source-controlled
    // optionalDependencies were removed for that reason — they'd drift
    // forever vs the published state).
    //
    // The release workflow has to build a binary for each target in
    // that list, or the published optionalDependencies point at
    // packages whose .node files never shipped. That silent mismatch
    // is the real regression risk; this test catches it by asserting
    // every entry in napi.targets shows up as a `--target <triple>`
    // in the release workflow's `build` commands.
    const nativePackage = (await readJson(
      "../../../crates/uncontainerizable-node/package.json"
    )) as {
      napi: { targets: string[] };
    };
    const releaseWorkflow = await readText(
      "../../../.github/workflows/release.yml"
    );

    for (const target of nativePackage.napi.targets) {
      expect(releaseWorkflow).toContain(`--target ${target}`);
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
