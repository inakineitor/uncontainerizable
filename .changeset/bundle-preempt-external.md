---
"uncontainerizable": patch
"@uncontainerizable/native": patch
---

Fix Launch Services bundle preemption so it also kills externally-launched instances of the bundle. Previously, passing an `identity` on a `.app` launch only terminated processes this supervisor had spawned itself (via a per-identity PID file). A Photoshop opened by double-click or another tool stayed alive when you asked uncontainerizable to launch a fresh one, which contradicted the "clean slate" intent of `identity`.

Bundle preemption now scans `ps` for every process whose executable matches the bundle's main-exec path (`<bundle>/Contents/MacOS/<CFBundleExecutable>`) and SIGKILLs each tree before `open -n` fires. Launches without `identity` are unchanged: no preemption, instances coexist. The PID-file mechanism in `~/Library/Caches/uncontainerizable/` is gone; `ps` is now the single source of truth.

Tradeoff worth noting: two distinct identities for the same bundle can no longer coexist on this path. That was never a supported use case, but it's spelled out in the module docs now.
