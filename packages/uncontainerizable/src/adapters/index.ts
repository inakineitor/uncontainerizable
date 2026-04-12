import { appkit } from "#/adapters/appkit.js";
import { chromium } from "#/adapters/chromium.js";
import { crashReporter } from "#/adapters/crash-reporter.js";
import { firefox } from "#/adapters/firefox.js";
import type { Adapter } from "#/types.js";

export { appkit } from "#/adapters/appkit.js";
export { chromium } from "#/adapters/chromium.js";
export { crashReporter } from "#/adapters/crash-reporter.js";
export { firefox } from "#/adapters/firefox.js";

/**
 * The bundle a typical browser-supervising caller wants: every built-in
 * adapter, in an order that doesn't matter (hook invocation is
 * idempotent and non-overlapping across adapters).
 */
export const defaultAdapters: readonly Adapter[] = [
  chromium,
  firefox,
  appkit,
  crashReporter,
];
