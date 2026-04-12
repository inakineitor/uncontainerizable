import { basename } from "node:path";

import type { Adapter, Probe } from "#/types.js";

const FIREFOX_BASENAMES = ["firefox", "Firefox", "firefox-bin"];

/**
 * Stub adapter for Firefox.
 *
 * Matches by executable basename. The real `clearCrashState` deletes
 * the profile's `sessionstore-backups/recovery.jsonlz4` so the next
 * launch does not offer session restore. Per-OS profile-dir detection
 * and lz4 handling ship in a follow-up.
 *
 * Until then `clearCrashState` emits a warning and returns.
 */
export const firefox: Adapter = {
  name: "firefox",

  matches(probe: Probe): boolean {
    const exe = probe.executablePath
      ? basename(probe.executablePath)
      : undefined;
    return exe ? FIREFOX_BASENAMES.includes(exe) : false;
  },

  clearCrashState(_probe: Probe): void {
    console.warn(
      "uncontainerizable: firefox.clearCrashState is a stub; the next launch may prompt to restore the previous session. Track the real implementation in a follow-up release."
    );
  },
};
