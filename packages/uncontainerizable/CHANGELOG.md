# uncontainerizable

## 0.1.2

### Patch Changes

- 05be3e0: Fix Launch Services bundle preemption so it also kills externally-launched instances of the bundle. Previously, passing an `identity` on a `.app` launch only terminated processes this supervisor had spawned itself (via a per-identity PID file). A Photoshop opened by double-click or another tool stayed alive when you asked uncontainerizable to launch a fresh one, which contradicted the "clean slate" intent of `identity`.

  Bundle preemption now scans `ps` for every process whose executable matches the bundle's main-exec path (`<bundle>/Contents/MacOS/<CFBundleExecutable>`) and SIGKILLs each tree before `open -n` fires. Launches without `identity` are unchanged: no preemption, instances coexist. The PID-file mechanism in `~/Library/Caches/uncontainerizable/` is gone; `ps` is now the single source of truth.

  PID resolution also takes a fresh post-preemption snapshot immediately before `open` launches the replacement. That prevents the resolver from attaching to a surviving or concurrently-started old instance that appeared during the reap-settle window.

  Tradeoff worth noting: two distinct identities for the same bundle can no longer coexist on this path. That was never a supported use case, but it's spelled out in the module docs now.

- Updated dependencies [05be3e0]
  - @uncontainerizable/native@0.1.2

## 0.1.1

### Patch Changes

- Added appropriate README.md to all published packages.
- Updated dependencies
  - @uncontainerizable/native@0.1.1

## 0.1.0

### Minor Changes

- 3e4dad2: Add Linux and Windows platform implementations.

  Linux uses cgroup v2 delegation: identity preemption is cgroup creation atomicity, and the quit ladder freezes the cgroup before signalling to deliver SIGTERM/SIGKILL race-free. On kernels with `cgroup.kill` (5.14+) the terminal stage writes `1` to that file for an atomic subtree kill.

  Windows uses Named Job Objects at `Local\uncontainerizable-<identity>`: identity preemption is atomic open-by-name, and dropping the supervisor handle triggers `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so orphans cannot survive a supervisor crash. The quit ladder posts `WM_CLOSE` to top-level windows, then terminates the job.

- 216b980: Add a Launch Services spawn path for macOS `.app` bundles. When the `command` passed to `App.contain` ends in `.app` and resolves to a directory, the library now shells out to `open -n -F -a <bundle>` instead of `posix_spawn`. This gets the app properly registered with `launchservicesd`: Dock icon, bundle-ID-addressable Apple Events, no stale "Reopen windows?" prompt on the next launch.

  Any other path (including executables inside a bundle like `Foo.app/Contents/MacOS/Foo`) continues through the existing direct-exec path, unchanged. Detection is purely based on path shape, no walking up to find an enclosing bundle, no new `ContainOptions` flag.

  Identity preemption on the Launch Services path is bundle-scoped: the library scans `ps` for every process whose executable matches the bundle's main executable path and kills each matching tree before `open -n` launches the replacement.

### Patch Changes

- Updated dependencies [3e4dad2]
- Updated dependencies [216b980]
  - @uncontainerizable/native@0.1.0

## 0.0.3

### Patch Changes

- Fix CI permissioning
- Updated dependencies
  - @uncontainerizable/native@0.0.3

## 0.0.2

### Patch Changes

- Align with rust crate version.
- Updated dependencies
  - @uncontainerizable/native@0.0.2

## 0.0.1

### Patch Changes

- First version
- Updated dependencies
  - @uncontainerizable/native@0.0.1
