import { basename } from "node:path";

import type { Adapter, Probe } from "#/types.js";

const CHROMIUM_BASENAMES = [
  "chrome",
  "chromium",
  "Chromium",
  "Google Chrome",
  "brave",
  "Brave Browser",
  "msedge",
  "Microsoft Edge",
];

/**
 * Stub adapter for Chromium-family browsers.
 *
 * Matches by executable basename (case-sensitive, since bundle exe names
 * are preserved verbatim). The real `clearCrashState` implementation
 * edits the "Last Exit Type" key in the browser's `Preferences` JSON so
 * the next launch does not prompt the user to restore a crashed
 * session. That implementation needs per-OS profile-dir detection and
 * careful JSON editing; it ships in a follow-up.
 *
 * Until then `clearCrashState` emits a warning and returns. The adapter
 * still matches so callers wiring it in can see the "didn't run" signal
 * and swap in a real cleanup when the full version ships.
 */
export const chromium: Adapter = {
  name: "chromium",

  matches(probe: Probe): boolean {
    const exe = probe.executablePath
      ? basename(probe.executablePath)
      : undefined;
    return exe ? CHROMIUM_BASENAMES.includes(exe) : false;
  },

  clearCrashState(_probe: Probe): void {
    console.warn(
      "uncontainerizable: chromium.clearCrashState is a stub; the next launch may show a 'didn't shut down correctly' dialog. Track the real implementation in a follow-up release."
    );
  },
};
