---
"uncontainerizable": minor
"@uncontainerizable/native": minor
---

Add Linux and Windows platform implementations.

Linux uses cgroup v2 delegation: identity preemption is cgroup creation atomicity, and the quit ladder freezes the cgroup before signalling to deliver SIGTERM/SIGKILL race-free. On kernels with `cgroup.kill` (5.14+) the terminal stage writes `1` to that file for an atomic subtree kill.

Windows uses Named Job Objects at `Local\uncontainerizable-<identity>`: identity preemption is atomic open-by-name, and dropping the supervisor handle triggers `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so orphans cannot survive a supervisor crash. The quit ladder posts `WM_CLOSE` to top-level windows, then terminates the job.
