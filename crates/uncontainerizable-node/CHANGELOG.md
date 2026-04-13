# @uncontainerizable/native

## 0.1.0

### Minor Changes

- 3e4dad2: Add Linux and Windows platform implementations.

  Linux uses cgroup v2 delegation: identity preemption is cgroup creation atomicity, and the quit ladder freezes the cgroup before signalling to deliver SIGTERM/SIGKILL race-free. On kernels with `cgroup.kill` (5.14+) the terminal stage writes `1` to that file for an atomic subtree kill.

  Windows uses Named Job Objects at `Local\uncontainerizable-<identity>`: identity preemption is atomic open-by-name, and dropping the supervisor handle triggers `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so orphans cannot survive a supervisor crash. The quit ladder posts `WM_CLOSE` to top-level windows, then terminates the job.

- 216b980: Add a Launch Services spawn path for macOS `.app` bundles. When the `command` passed to `App.contain` ends in `.app` and resolves to a directory, the library now shells out to `open -n -F -a <bundle>` instead of `posix_spawn`. This gets the app properly registered with `launchservicesd`: Dock icon, bundle-ID-addressable Apple Events, no stale "Reopen windows?" prompt on the next launch.

  Any other path (including executables inside a bundle like `Foo.app/Contents/MacOS/Foo`) continues through the existing direct-exec path, unchanged. Detection is purely based on path shape — no walking up to find an enclosing bundle, no new `ContainOptions` flag.

  Identity preemption on the Launch Services path is PID-file based under `~/Library/Caches/uncontainerizable/`. The file stores the last spawned PID per identity and is checked + overwritten on each spawn, so it survives supervisor crashes.

## 0.0.3

### Patch Changes

- Fix CI permissioning

## 0.0.2

### Patch Changes

- Align with rust crate version.

## 0.0.1

### Patch Changes

- First version
