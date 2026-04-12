import { readdir, rm, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, join } from "node:path";

import type { Adapter, Probe } from "#/types.js";

/**
 * Deletes per-app crash reports from macOS's user-level CrashReporter
 * archive after a terminal-stage destroy.
 *
 * When our SIGTERM/SIGKILL ladder kills an app, macOS's `ReportCrash`
 * daemon writes an `.ips` entry under
 * `~/Library/Logs/DiagnosticReports/` named like
 * `<AppName>_<timestamp>_<host>.ips`. The next time the user opens the
 * Console app (or the crash-reporter dialog appears) those entries
 * show up as "<App> quit unexpectedly" even though the quit was
 * deliberate.
 *
 * This adapter matches by the basename of `Probe.executablePath` and
 * deletes every report whose filename starts with that basename,
 * followed by an underscore. We only touch files under the user's
 * DiagnosticReports directory; the system-wide `/Library/Logs/...`
 * archive is root-owned and out of scope.
 *
 * Note that this does NOT suppress the modal "quit unexpectedly"
 * dialog that fires immediately when a process crashes: that is
 * displayed before any cleanup hook runs. For that case, let the
 * `apple_event_quit` stage resolve the quit cleanly instead of
 * escalating to SIGTERM/SIGKILL.
 */
export const crashReporter: Adapter = {
  name: "crashReporter",

  matches(probe: Probe): boolean {
    return (
      probe.platform === "darwin" && typeof probe.executablePath === "string"
    );
  },

  async clearCrashState(probe: Probe): Promise<void> {
    if (!probe.executablePath) {
      return;
    }
    const appName = basename(probe.executablePath);
    if (!appName) {
      return;
    }
    const dir = join(homedir(), "Library", "Logs", "DiagnosticReports");
    // `fs.stat` rejects if the path doesn't exist. A fresh user account
    // may not have a DiagnosticReports directory, so probe first and
    // short-circuit if it's missing or not a directory. Swallowing the
    // reject into `undefined` keeps the happy path straight-line.
    const stats = await stat(dir).catch(() => undefined);
    if (!stats?.isDirectory()) {
      return;
    }
    const entries = await readdir(dir);
    const prefix = `${appName}_`;
    const removals = entries
      .filter(
        (name) =>
          name.startsWith(prefix) &&
          (name.endsWith(".ips") || name.endsWith(".crash"))
      )
      .map((name) => rm(join(dir, name), { force: true }));
    await Promise.all(removals);
  },
};
