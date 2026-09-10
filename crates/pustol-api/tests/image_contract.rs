//! The image and the workflow that publishes it, as text.
//!
//! Infra units hard-depend on paths, the missing ENTRYPOINT, and an SSH command that is exactly
//! `pustol`. A container we have not built yet cannot speak; the files that produce it can.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the crate lives in this repository")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("missing {relative}: {error}"))
}

fn dockerfile_instructions(text: &str) -> Vec<String> {
    let mut instructions = Vec::new();
    let mut current = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let continued = line.ends_with('\\');
        let piece = line.trim_end_matches('\\').trim();
        if current.is_empty() {
            piece.clone_into(&mut current);
        } else {
            current.push(' ');
            current.push_str(piece);
        }
        if !continued {
            instructions.push(std::mem::take(&mut current));
        }
    }
    instructions
}

fn last_stage(instructions: &[String]) -> &[String] {
    let start = instructions
        .iter()
        .enumerate()
        .filter(|(_, line)| line.to_ascii_uppercase().starts_with("FROM "))
        .map(|(index, _)| index)
        .next_back()
        .expect("a runtime FROM");
    &instructions[start..]
}

#[test]
fn dockerfile_matches_the_image_contract() {
    let instructions = dockerfile_instructions(&read("Dockerfile"));
    let runtime = last_stage(&instructions);

    assert!(
        !runtime
            .iter()
            .any(|line| line.to_ascii_uppercase().starts_with("ENTRYPOINT ")),
        "an ENTRYPOINT makes `podman run … seed` arguments to pustol-api and first-boot crash-loops"
    );
    assert!(
        runtime.iter().any(|line| line == "CMD [\"pustol-api\"]"),
        "runtime CMD must be [\"pustol-api\"], got {runtime:?}"
    );
    assert!(
        runtime
            .iter()
            .any(|line| line.eq_ignore_ascii_case("STOPSIGNAL SIGINT")),
        "STOPSIGNAL SIGINT is missing from the runtime stage"
    );
    assert!(
        instructions
            .iter()
            .any(|line| { line.contains("/usr/local/bin/pustol-api") && !line.contains("seed") }),
        "pustol-api must land at /usr/local/bin/pustol-api"
    );
    assert!(
        instructions
            .iter()
            .any(|line| line.contains("/usr/local/bin/seed")),
        "seed must land at /usr/local/bin/seed"
    );
    assert!(
        runtime
            .iter()
            .any(|line| line.contains("PUSTOL_ASSETS_DIR=/var/lib/pustol/web")),
        "PUSTOL_ASSETS_DIR must be baked to /var/lib/pustol/web"
    );
    assert!(
        runtime.iter().any(|line| line.contains("ca-certificates")),
        "runtime image must install ca-certificates so rustls can reach api.telegram.org"
    );
    assert!(
        runtime.iter().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("curl")
        }),
        "runtime image must install curl; infra probes with curl -fsS http://localhost:8080/health"
    );
    assert!(
        !runtime.iter().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("node") || lower.contains("npm")
        }),
        "Node is build-only; the runtime stage mentions it: {runtime:?}"
    );
    assert!(
        instructions
            .iter()
            .any(|line| line.starts_with("ARG NEXT_PUBLIC_BOT_USERNAME")),
        "NEXT_PUBLIC_BOT_USERNAME is required at web build and must be a build-arg"
    );
}

#[test]
fn release_workflow_sshes_exactly_pustol() {
    let workflow = read(".github/workflows/release-api.yml");
    assert!(
        workflow.contains("contents: read"),
        "workflow must request contents: read"
    );
    assert!(
        workflow.contains("packages: write"),
        "workflow must request packages: write"
    );
    assert!(
        workflow.contains("secrets.DEPLOY_KEY"),
        "deploy authenticates with DEPLOY_KEY"
    );
    assert!(
        workflow.contains("secrets.KNOWN_HOSTS"),
        "deploy verifies the host with KNOWN_HOSTS"
    );
    assert!(
        !workflow.contains("VPS_HOST"),
        "VPS_HOST is the old mistake: the host comes from KNOWN_HOSTS, not a mutating-script address"
    );
    assert!(
        workflow.contains("vars.NEXT_PUBLIC_BOT_USERNAME"),
        "bot username is a repo variable baked at build, not a runtime secret on the VPS"
    );
    assert!(
        workflow.contains(":sha-") && workflow.contains(":prod"),
        "tags must include immutable sha-… and moving :prod"
    );
    assert!(
        workflow.contains("rollback_sha"),
        "workflow_dispatch rollback retags :prod to a known sha"
    );
    assert!(
        workflow.contains("environment: production"),
        "the release job must use the production environment so NEXT_PUBLIC_BOT_USERNAME is qbqs_bot"
    );
    assert!(
        !workflow.contains("DEPLOY_USER"),
        "DEPLOY_USER hides a missing SSH port behind a missing user; SSH as deploy on 7759"
    );
    assert!(
        workflow.contains("-p 7759"),
        "SSH without -p 7759 hits port 22 and dies"
    );
    assert!(
        workflow.contains("deploy@${host}"),
        "SSH user is deploy, not a repo variable"
    );
    // The remote argv is exactly the unit name. A here-doc or `bash -c` would not be the
    // forced command infra installed.
    assert!(
        workflow
            .lines()
            .any(|line| line.trim() == "pustol" || line.trim().ends_with(" pustol")),
        "the SSH command must be exactly `pustol`"
    );
}
