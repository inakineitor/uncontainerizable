# Changesets

This folder tracks versioning intent for the `uncontainerizable` and
`@uncontainerizable/native` packages. The two packages are declared `linked` in
`config.json`, so they always publish at the same version. Mismatched versions
cause runtime `require` failures for consumers of the TS wrapper.

## Adding a changeset

Run `pnpm changeset` from the repo root, describe the change, and commit the
resulting markdown file. CI opens a "Version Packages" PR on merges to `main`;
merging that PR publishes to npm via `changesets/action`.
