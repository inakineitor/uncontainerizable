---
"uncontainerizable": minor
"@uncontainerizable/native": minor
---

Add a Launch Services spawn path for macOS `.app` bundles. When the `command` passed to `App.contain` ends in `.app` and resolves to a directory, the library now shells out to `open -n -F -a <bundle>` instead of `posix_spawn`. This gets the app properly registered with `launchservicesd`: Dock icon, bundle-ID-addressable Apple Events, no stale "Reopen windows?" prompt on the next launch.

Any other path (including executables inside a bundle like `Foo.app/Contents/MacOS/Foo`) continues through the existing direct-exec path, unchanged. Detection is purely based on path shape — no walking up to find an enclosing bundle, no new `ContainOptions` flag.

Identity preemption on the Launch Services path is PID-file based under `~/Library/Caches/uncontainerizable/`. The file stores the last spawned PID per identity and is checked + overwritten on each spawn, so it survives supervisor crashes.
