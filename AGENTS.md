# Agent instructions

This file provides guidance to coding agents when working with code in this repository.

## What this is

`uncontainerizable` is a supervisor library for programs that can't be put in real containers (browsers, GUI apps, anything needing the user's window server/keychain/display). Pure-Rust core + napi-rs Node bindings, published as `uncontainerizable` (TS wrapper) and `@uncontainerizable/native` (bindings).

Design reference: **`documents/development-plan.md`**. It is authoritative for concepts, invariants, per-platform mechanisms, and the expected module layout. Read it before making nontrivial changes to the core or bindings.

## Workspace topology

Three publishable units and one private tsconfig package:

- `crates/uncontainerizable-core/`: pure Rust lib (the engine; not published).
- `crates/uncontainerizable-node/`: `cdylib` with napi-rs bindings. **This crate is simultaneously a pnpm workspace member** (see `pnpm-workspace.yaml`). Its `package.json` ships as `@uncontainerizable/native`.
- `packages/uncontainerizable/`: TS wrapper that re-exports from `@uncontainerizable/native`. Depends on it via `workspace:*`.
- `packages/tsconfig/`: shared `@uncontainerizable/tsconfig/base.json`, workspace-private.

The two publishable packages are **linked in Changesets** (`.changeset/config.json`). They must always ship at the same version. Mismatches cause runtime `require` failures for consumers.

## Build & task commands

```sh
pnpm install              # pnpm 10.x, see packageManager in package.json
pnpm build                # Full build via turbo; native first, then TS, in topological order
pnpm build:native         # Only the napi-rs build
pnpm build:ts             # Only tsdown; depends on ^build (upstream native)
pnpm test                 # Vitest across TS packages
pnpm typecheck            # tsc --noEmit per TS package
pnpm lint                 # Ultracite check (TS/JS/JSON)
pnpm lint:fix             # Ultracite fix
pnpm lint:rust            # cargo fmt --check + cargo clippy -D warnings
pnpm lint:package         # publint against every publishable package

cargo check --workspace   # Rust-only compile check
cargo test --workspace --exclude uncontainerizable-node
pnpm --filter uncontainerizable test -- <pattern>   # single vitest file/name
```

**`build:native` must run before `build:ts` in the same pnpm install.** The TS wrapper is not a stub; its `@uncontainerizable/native` dep resolves to a built `.node` + loader at `crates/uncontainerizable-node/dist/`. If you've modified the Rust surface, rebuild native before running typecheck or vitest.

## napi-rs output layout (non-default)

The native crate builds into `crates/uncontainerizable-node/dist/`, not the crate root:

```
napi build --platform --release --output-dir dist --js index.js --dts index.d.ts
```

Output is `dist/index.js` (loader), `dist/index.d.ts`, `dist/<binaryName>.<platform>.node`. `package.json` `main`/`types`/`files` point at `./dist/...`. If you change this, update `turbo.json` outputs and the release workflow's `upload-artifact` path together, since both reference this directory explicitly.

## Platform dispatch (core)

Per the dev plan, `platforms::spawn(app, command, opts)` is the single entry point into platform code. It `cfg`-dispatches to `linux`, `darwin`, or `win32`. Each platform owns its own `stages`, `probe`, and preemption primitive:

- **Linux**: cgroup v2 at `{session}/uncontainerizable/{identity}/`. Identity preemption uses `mkdir` atomicity; teardown uses `cgroup.freeze` for race-free SIGKILL.
- **Windows**: Named Job Object `Local\uncontainerizable-{identity}`. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` guarantees cleanup if the supervisor dies.
- **macOS**: Direct-exec is best-effort via `argv[0]` tagging (`uncontainerizable:{identity}/original-name`), and callers can opt out via `ContainOptions.darwin_tag_argv0 = false`. Launch Services `.app` launches instead use bundle-scoped preemption via `ps comm=` against the bundle executable path, so they cannot keep two instances of the same bundle alive concurrently. Use the inner executable path only if the app itself supports concurrent instances.

Adapter hooks are **advisory**: errors are collected into the final result, never abort escalation. `destroy()` is infallible (never throws; aggregates errors). See `documents/development-plan.md` "Invariants" for the full list.

## TypeScript conventions (TS 6.0)

- Source lives in `packages/uncontainerizable/src/`. Internal imports use the **subpath import** pattern: `import type { X } from "#/types.js"` resolves to `./src/types.ts` via `"imports": { "#/*": "./src/*" }` in the wrapper's `package.json`.
- Base tsconfig is intentionally opinionated for TS 6:
  - `target`/`lib`: `ESNext`. The wrapper uses `noEmit: true`, so tsc's `target` doesn't control emit (tsdown/rolldown does, with its own `target: "node24"` in `tsdown.config.ts`). Runtime API availability is fenced by `@types/node` + `engines.node` (`>=24`, current LTS) + CI, not by a narrower `lib`.
  - `types: []` in base; packages must declare `types: ["node"]` (TS 6 no longer auto-loads all `@types/*`).
  - `moduleResolution: "NodeNext"`, `moduleDetection: "force"`, `verbatimModuleSyntax: true`.
- Wrapper ships dual CJS/ESM via tsdown; `tsdown.config.ts` externalizes `@uncontainerizable/native` so the bundler doesn't try to inline the loader.

## Quality gates

Biome (via Ultracite preset) handles TS/JS/JSON/CSS formatting and linting. **Don't add ESLint or Prettier.** Root config extends `ultracite/biome/core` + `ultracite/biome/vitest`. Two Ultracite rules are disabled repo-wide because they fight library ergonomics: `performance/noBarrelFile` and `performance/noReExportAll` (the wrapper's entry points re-export by design).

Rust: `rustfmt` + `clippy` with `-D warnings`. Lefthook runs both plus Ultracite on `pre-commit`, and commitlint (Conventional Commits) on `commit-msg`. Commits must be conventional.

`publint` runs as `lint:package` per publishable package and in CI; its warnings fail the build. Treat them as errors, not suggestions.

## CI / release

- `.github/workflows/ci.yml` on PR + push to `main`: lint (Ultracite + cargo fmt/clippy + publint), rust test matrix (ubuntu/macos/windows), node test matrix (same, with a native build step per runner).
- `.github/workflows/release.yml` on push to `main`: 8-target `build-native` matrix (macOS x64/arm64, Linux gnu/musl x64/arm64 via `cargo-zigbuild`+zig for musl, Windows x64/arm64). `napi artifacts` assembles `npm/` per-platform packages; `changesets/action` opens a Version Packages PR or publishes. `NPM_CONFIG_PROVENANCE=true` is set.

## Things that will bite you

- Changing the `napi` field in `crates/uncontainerizable-node/package.json` (`binaryName` or `targets`) must be mirrored in the release workflow matrix. They are not auto-generated from each other.
- The `.node` binary is platform-specific. A local `pnpm build` only produces the host target; cross-target binaries only exist after CI.
- Ultracite v7's `extends` uses submodule paths (`ultracite/biome/core`), not the v6 `"ultracite"` shorthand. Don't "fix" this.
- `Cargo.lock` is gitignored (library workspace convention). CI regenerates; commit it if you need fully reproducible Rust builds.

## Writing style

- Do not use em dashes. Rewrite sentences with a period, comma, colon, or parentheses instead.
- Use semicolons sparingly. Prefer two short sentences over one sentence joined by a semicolon. Reserve semicolons for cases where the second clause genuinely extends the first and a period would feel too abrupt.

## Feature workflow

This repo uses Jujutsu (`jj`) colocated with Git, plus `jj-spr` for stacked
pull requests. Each feature lives in its own secondary workspace so that the
default workspace stays free for unrelated work. The steps below assume the
primary clone is already set up as a colocated jj repo and `jj spr init` has
been run once.

### 1. Create a workspace rooted on `main@origin`

```bash
jj git fetch
jj workspace add -r main@origin ../<repo-name>-<feature-slug>
cd ../<repo-name>-<feature-slug>
```

`jj workspace add` creates a sibling directory with its own working copy that
shares the underlying jj repo. The new `@` starts on `main@origin`, so the
feature work will not be accidentally based on in-progress changes from the
default workspace.

### 2. Work on the feature

Describe the change up front and keep `@` as a small empty scratch commit on
top. Always pass `-m` to avoid opening an editor mid-task.

```bash
jj describe -m "feat(area): short title

Longer body if needed.

Reviewers: <github-username>"
jj new -m "wip"
# make code changes, run `pnpm typecheck`, etc.
jj squash            # fold @ into the described change at @-
```

Rebase onto the latest trunk whenever `main@origin` moves:

```bash
jj git fetch
jj rebase -s <change-id> -d main@origin
```

### 3. Open the PR with `jj-spr`

Before creating the PR, fetch and rebase onto `main@origin` so that the PR
is opened against the current tip of trunk. Skipping this step can produce a
PR diff that includes unrelated upstream work or appears to conflict when it
does not.

```bash
jj git fetch
jj rebase -s <change-id> -d main@origin
```

`jj-spr` drives Git directly, so it must run from the colocated (default)
workspace. Preparation (edits, describe, rebase, typecheck) happens in the
feature workspace. Only the push step needs to be invoked from the default
workspace.

From the default workspace, referencing the feature change by its change ID:

```bash
jj spr diff -r <change-id>
```

For later pushes on the same change, pass `-m` so `jj-spr` does not prompt:

```bash
jj spr diff -r <change-id> -m "rebase onto latest main"
jj spr diff -r <change-id> -m "address review feedback"
```

Local `jj describe` edits do not push. To sync the commit message with
GitHub, run `jj spr diff --update-message -r <change-id>`. To pull a GitHub
side edit back into the local change, run `jj spr amend -r <change-id>`.

**Do not clobber the `Pull Request:` trailer.** After `jj spr diff` creates
a PR, it appends a `Pull Request: https://github.com/<org>/<repo>/pull/<n>`
trailer to the change's description. `jj-spr` uses that trailer to locate
the existing PR on every subsequent `jj spr diff`. If the trailer is
missing, `jj-spr` assumes this is a new change and opens a fresh PR,
leaving the original one orphaned.

A naive `jj describe -r <change-id> -m "..."` replaces the entire
description, stripping the trailer. Do not do that. Instead:

- To **amend code** (no message change): make the edits, `jj squash` into
  the described change, and re-run `jj spr diff -r <change-id> -m "what
changed"`. The trailer is preserved because the description is untouched.
- To **update the commit message**: edit the local description however
  you like, then run `jj spr diff --update-message -r <change-id>`. That
  command pushes the new message to the existing PR and re-applies the
  trailer locally. Do not follow a `jj describe` with a plain
  `jj spr diff`; that opens a new PR.
- To **pull a GitHub-side message edit back** (e.g. a reviewer fixed a
  typo in the PR description): run `jj spr amend -r <change-id>`.
- Before rewriting a description, confirm the trailer is already present
  with `jj show <change-id>`. If it is, preserve it verbatim in the new
  message, or use `--update-message` instead of a plain describe.

If you accidentally strip the trailer and a duplicate PR is opened, close
the stale PR (`gh pr close <old>`) and continue on the new one. Do not
try to re-point the trailer manually; just fix the workflow going forward.

### 4. Babysit the PR

Submitting the PR is not the end of the task. CI runs asynchronously after
the push, so the workflow is only complete once every check is green. Watch
the run, fix any failures, and re-push until CI passes.

Useful commands (run from the default workspace):

```bash
# One-shot status of all checks on a PR
gh pr checks <pr-number>

# Poll until the run completes, then exit with the run's status
gh pr checks <pr-number> --watch

# Drill into a specific failed job's log
gh run view <run-id> --log-failed
```

When a check fails:

1. Reproduce the failure locally in the feature workspace (for example,
   `pnpm ultracite check`, `pnpm typecheck`, or the specific test command).
2. Fix the issue, `jj squash` the fix into the feature change so history
   stays clean, and run the local command again to confirm it now passes.
3. Push the updated change with
   `jj spr diff -r <change-id> -m "fix: <what was fixed>"`.
4. Re-check CI. Do not land the PR until every required check is green.

Do not disable, skip, or ignore checks to get a PR green. Fix the underlying
issue, or raise it with the team if the check itself is wrong.

### 5. After the PR merges, clean up the workspace

Once the PR is merged on GitHub:

```bash
# From either workspace, pick up the landed commit
jj git fetch

# Unregister the feature workspace from the jj repo
jj workspace forget <repo-name>-<feature-slug>

# Delete the working directory from disk
rm -rf ../<repo-name>-<feature-slug>
```

`jj workspace forget` only removes the repo's reference to the workspace. The
directory must be deleted separately. Skipping `forget` leaves an orphaned
workspace entry in the jj repo metadata.

Do not touch other workspaces during cleanup. Another agent may be actively
working in them, and rebasing their `@` would disrupt that work.

## When these instructions are wrong

If any instruction in this file is out of date, contradicts itself, or does
not apply to the current task, do not silently work around it. Stop, propose
a concrete fix to `AGENTS.md`, and ask the user whether they want to apply
the proposed edit before continuing. Keeping the file accurate over time is
more valuable than finishing one task slightly faster.

## Coding standards

This project uses **Ultracite**, a zero-config preset that enforces strict code quality standards through automated formatting and linting.

### Quick Reference

- **Format code**: `pnpm dlx ultracite fix`
- **Check for issues**: `pnpm dlx ultracite check`
- **Diagnose setup**: `pnpm dlx ultracite doctor`

Biome (the underlying engine) provides robust linting and formatting. Most issues are automatically fixable.

---

### Core Principles

Write code that is **accessible, performant, type-safe, and maintainable**. Focus on clarity and explicit intent over brevity.

#### Type Safety & Explicitness

- Use explicit types for function parameters and return values when they enhance clarity
- Prefer `unknown` over `any` when the type is genuinely unknown
- Use const assertions (`as const`) for immutable values and literal types
- Leverage TypeScript's type narrowing instead of type assertions
- Use meaningful variable names instead of magic numbers - extract constants with descriptive names

#### Modern JavaScript/TypeScript

- Use arrow functions for callbacks and short functions
- Prefer `for...of` loops over `.forEach()` and indexed `for` loops
- Use optional chaining (`?.`) and nullish coalescing (`??`) for safer property access
- Prefer template literals over string concatenation
- Use destructuring for object and array assignments
- Use `const` by default, `let` only when reassignment is needed, never `var`

#### Async & Promises

- Always `await` promises in async functions - don't forget to use the return value
- Use `async/await` syntax instead of promise chains for better readability
- Handle errors appropriately in async code with try-catch blocks
- Don't use async functions as Promise executors

#### React & JSX

- Use function components over class components
- Call hooks at the top level only, never conditionally
- Specify all dependencies in hook dependency arrays correctly
- Use the `key` prop for elements in iterables (prefer unique IDs over array indices)
- Nest children between opening and closing tags instead of passing as props
- Don't define components inside other components
- Use semantic HTML and ARIA attributes for accessibility:
  - Provide meaningful alt text for images
  - Use proper heading hierarchy
  - Add labels for form inputs
  - Include keyboard event handlers alongside mouse events
  - Use semantic elements (`<button>`, `<nav>`, etc.) instead of divs with roles

#### Error Handling & Debugging

- Remove `console.log`, `debugger`, and `alert` statements from production code
- Throw `Error` objects with descriptive messages, not strings or other values
- Use `try-catch` blocks meaningfully - don't catch errors just to rethrow them
- Prefer early returns over nested conditionals for error cases

#### Code Organization

- Keep functions focused and under reasonable cognitive complexity limits
- Extract complex conditions into well-named boolean variables
- Use early returns to reduce nesting
- Prefer simple conditionals over nested ternary operators
- Group related code together and separate concerns

#### Security

- Add `rel="noopener"` when using `target="_blank"` on links
- Avoid `dangerouslySetInnerHTML` unless absolutely necessary
- Don't use `eval()` or assign directly to `document.cookie`
- Validate and sanitize user input

#### Performance

- Avoid spread syntax in accumulators within loops
- Use top-level regex literals instead of creating them in loops
- Prefer specific imports over namespace imports
- Avoid barrel files (index files that re-export everything)
- Use proper image components (e.g., Next.js `<Image>`) over `<img>` tags

#### Framework-Specific Guidance

**Next.js:**

- Use Next.js `<Image>` component for images
- Use `next/head` or App Router metadata API for head elements
- Use Server Components for async data fetching instead of async Client Components

**React 19+:**

- Use ref as a prop instead of `React.forwardRef`

**Solid/Svelte/Vue/Qwik:**

- Use `class` and `for` attributes (not `className` or `htmlFor`)

---

### Testing

- Write assertions inside `it()` or `test()` blocks
- Avoid done callbacks in async tests - use async/await instead
- Don't use `.only` or `.skip` in committed code
- Keep test suites reasonably flat - avoid excessive `describe` nesting

### When Biome Can't Help

Biome's linter will catch most issues automatically. Focus your attention on:

1. **Business logic correctness** - Biome can't validate your algorithms
2. **Meaningful naming** - Use descriptive names for functions, variables, and types
3. **Architecture decisions** - Component structure, data flow, and API design
4. **Edge cases** - Handle boundary conditions and error states
5. **User experience** - Accessibility, performance, and usability considerations
6. **Documentation** - Add comments for complex logic, but prefer self-documenting code

---

Most formatting and common issues are automatically fixed by Biome. Run `pnpm dlx ultracite fix` before committing to ensure compliance.
