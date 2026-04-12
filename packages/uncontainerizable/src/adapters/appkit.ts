import { rm } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

import type { Adapter, Probe } from "#/types.js";

/**
 * Deletes the AppKit "Saved Application State" directory for the managed
 * app after a terminal-stage destroy.
 *
 * macOS's AppKit persists a per-app snapshot of open windows and the
 * "should reopen windows on next launch" hint at
 * `~/Library/Saved Application State/<bundleId>.savedState/`. When an
 * app is force-killed (our SIGTERM/SIGKILL ladder), AppKit interprets
 * the missing clean-shutdown marker as a crash and shows the
 * "Reopen windows?" dialog next launch. Wiping the directory suppresses
 * the dialog.
 *
 * Only matches on darwin probes that carry a resolved `bundleId`.
 * Programs spawned without bundle-ID resolution (anything outside
 * `lsappinfo`'s knowledge of launched apps, for example `sleep` or a
 * manually-compiled binary) silently skip.
 */
export const appkit: Adapter = {
  name: "appkit",

  matches(probe: Probe): boolean {
    return probe.platform === "darwin" && typeof probe.bundleId === "string";
  },

  async clearCrashState(probe: Probe): Promise<void> {
    if (!probe.bundleId) {
      return;
    }
    const dir = join(
      homedir(),
      "Library",
      "Saved Application State",
      `${probe.bundleId}.savedState`
    );
    await rm(dir, { recursive: true, force: true });
  },
};
