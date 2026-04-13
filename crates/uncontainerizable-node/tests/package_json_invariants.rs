//! Invariants about this crate's `package.json`.
//!
//! `napi prepublish` regenerates the `optionalDependencies` block
//! from `napi.targets` immediately before `npm publish`, so any
//! source-committed `@uncontainerizable/native-*` entry drifts
//! against the generated state forever. Bumping the native crate's
//! `version` in a Changesets Version Packages PR would then need a
//! matching bump of eight sibling packages that don't yet exist on
//! npm — a lockfile pnpm can't resolve. Source must stay free of
//! self-referential entries so that pitfall can't recur.
//!
//! The `package.json` lives alongside this crate, so the invariant
//! lives here rather than in the TypeScript workspace config suite:
//! the check runs under `cargo test` in every Test Rust matrix job,
//! no new JavaScript tooling needed.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

const NATIVE_PACKAGE_PREFIX: &str = "@uncontainerizable/native-";
const DEP_FIELDS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

fn package_json() -> Value {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir.join("package.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn native_package_does_not_commit_its_own_optional_dependencies() {
    let pkg = package_json();
    for field in DEP_FIELDS {
        let Some(entries) = pkg.get(field).and_then(Value::as_object) else {
            continue;
        };
        for name in entries.keys() {
            assert!(
                !name.starts_with(NATIVE_PACKAGE_PREFIX),
                "{field} must not list {name}; napi prepublish generates \
                 optionalDependencies at publish time, so committed entries \
                 drift against the published manifest and break \
                 `pnpm install --frozen-lockfile` on Version Packages PRs"
            );
        }
    }
}

#[test]
fn napi_targets_are_declared() {
    let pkg = package_json();
    let targets = pkg
        .pointer("/napi/targets")
        .and_then(Value::as_array)
        .expect("package.json must declare napi.targets");
    assert!(
        !targets.is_empty(),
        "napi.targets drives optionalDependencies generation at publish; \
         an empty list would ship a native package with no .node binaries"
    );
    for target in targets {
        assert!(
            target.is_string(),
            "every napi.targets entry must be a triple string, got {target}"
        );
    }
}

#[test]
fn release_workflow_builds_every_napi_target() {
    let pkg = package_json();
    let targets = pkg
        .pointer("/napi/targets")
        .and_then(Value::as_array)
        .expect("package.json must declare napi.targets");

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf();
    let release_workflow_path = workspace_root
        .join(".github")
        .join("workflows")
        .join("release.yml");
    let workflow = fs::read_to_string(&release_workflow_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", release_workflow_path.display()));

    for target in targets {
        let triple = target.as_str().expect("napi.targets entry is a string");
        let needle = format!("--target {triple}");
        assert!(
            workflow.contains(&needle),
            "release workflow is missing a build job for napi target {triple}; \
             publishing without it would ship an optionalDependency whose .node \
             file never got uploaded"
        );
    }
}
