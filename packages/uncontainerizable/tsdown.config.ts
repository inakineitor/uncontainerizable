import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/index.ts", "src/adapters/index.ts"],
  format: ["cjs", "esm"],
  target: "node24",
  dts: true,
  clean: true,
  external: ["@uncontainerizable/native"],
});
