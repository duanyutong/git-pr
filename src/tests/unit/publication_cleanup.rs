use super::*;
use crate::tests::unit::support::{Response, World};

const DESTINATION: &str = "https://github.com/org/repo.git";
const TEMPORARY: &str = "git-pr/transition/101";

fn oid(number: u64) -> String {
    format!("{number:040x}")
}

fn fixture() -> std::sync::Arc<World> {
    let world = World::new(1);
    world.0.lock().unwrap().remote_refs.extend([
        ("refs/heads/main".into(), oid(0)),
        (format!("refs/heads/{TEMPORARY}"), oid(0)),
    ]);
    world.reply(
        &format!("git merge-base --is-ancestor {} {}", oid(0), oid(0)),
        0,
        "",
    );
    world
}

fn cleanup(world: &std::sync::Arc<World>, candidates: &BTreeMap<String, String>) -> Result<()> {
    let config = world.config();
    let commands = world.commands();
    remove_temporary(
        &GitHub {
            session: crate::github::Session::default(),
            config: &config,
            commands: &commands,
            clock: Some(world.as_ref()),
            transport: world.as_ref(),
            template: "",
        },
        DESTINATION,
        candidates,
        None,
    )
}

#[test]
fn deletion_is_leased_and_missing_or_empty_cleanup_requires_no_mutation() {
    let world = fixture();
    let candidates = BTreeMap::from([(TEMPORARY.into(), oid(0))]);
    cleanup(&world, &candidates).unwrap();
    assert!(world.trace().iter().any(|event| {
        event["args"][0] == "push"
            && event["args"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(format!(
                    "--force-with-lease=refs/heads/{TEMPORARY}:{}",
                    oid(0)
                )))
    }));
    world.0.lock().unwrap().trace.clear();
    cleanup(&world, &candidates).unwrap();
    assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
    world.0.lock().unwrap().trace.clear();
    cleanup(&world, &BTreeMap::new()).unwrap();
    assert!(!world.trace().iter().any(|event| event["args"][0] == "push"));
}

#[test]
fn incomplete_or_changed_cleanup_evidence_never_authorises_deletion() {
    for case in [
        "destination",
        "unavailable-destination",
        "refs",
        "tip",
        "trunk",
        "ancestry",
        "unavailable-ancestry",
        "dependants",
        "push",
    ] {
        let world = fixture();
        let candidates = BTreeMap::from([(TEMPORARY.into(), oid(0))]);
        match case {
            "destination" => world.reply("git remote get-url --push --all -- origin", 0, "git@github.com:org/repo.git"),
            "unavailable-destination" => world.fail("git remote get-url --push --all -- origin"),
            "refs" => world.fail(&format!("git ls-remote --refs -- {DESTINATION} refs/heads/{TEMPORARY}")),
            "tip" => { world.0.lock().unwrap().remote_refs.insert(format!("refs/heads/{TEMPORARY}"), oid(1)); }
            "trunk" => world.fail("git rev-parse --verify refs/remotes/origin/main^{commit}"),
            "ancestry" => world.reply(&format!("git merge-base --is-ancestor {} {}", oid(0), oid(0)), 1, ""),
            "unavailable-ancestry" => world.fail(&format!("git merge-base --is-ancestor {} {}", oid(0), oid(0))),
            "dependants" => world.http("POST graphql", vec![Response::Failure("dependants unavailable".into())]),
            "push" => world.fail(&format!("git push --porcelain --no-follow-tags --force-with-lease=refs/heads/{TEMPORARY}:{} -- origin :refs/heads/{TEMPORARY}", oid(0))),
            _ => unreachable!(),
        }
        assert!(cleanup(&world, &candidates).is_err(), "{case}");
        assert!(
            world
                .0
                .lock()
                .unwrap()
                .remote_refs
                .contains_key(&format!("refs/heads/{TEMPORARY}")),
            "{case}"
        );
        if case != "push" {
            assert!(
                !world.trace().iter().any(|event| event["args"][0] == "push"),
                "{case}"
            );
        }
    }
}

#[test]
fn cleanup_failure_and_retry_preserve_completed_publication_and_native_membership() {
    for native in [false, true] {
        let world = World::new(2);
        let flags = [if native {
            "--github-stack"
        } else {
            "--no-stack"
        }];
        world.run(&flags).unwrap();
        let original = world.0.lock().unwrap().prs.clone();
        world.0.lock().unwrap().remote_refs.extend([
            ("refs/heads/main".into(), oid(0)),
            ("refs/heads/independent".into(), oid(999)),
            (format!("refs/heads/{TEMPORARY}"), oid(0)),
        ]);
        world.reply(
            &format!("git merge-base --is-ancestor {} {}", oid(0), oid(0)),
            0,
            "",
        );
        let deletion = format!(
            "git push --porcelain --no-follow-tags --force-with-lease=refs/heads/{TEMPORARY}:{} -- origin :refs/heads/{TEMPORARY}",
            oid(0)
        );
        world.fail(&deletion);
        for failed in [true, false] {
            world.0.lock().unwrap().trace.clear();
            world.run(&flags).unwrap();
            let memory = world.0.lock().unwrap();
            assert_eq!(memory.prs, original);
            assert_eq!(memory.remote_refs["refs/heads/independent"], oid(999));
            for event in &memory.trace {
                assert!(!matches!(
                    event["method"].as_str(),
                    Some("PATCH" | "DELETE")
                ));
                assert!(event["method"] != "POST" || event["path"] == "graphql");
                if event["args"][0] == "push" {
                    assert_eq!(
                        event["args"].as_array().unwrap().last().unwrap(),
                        &format!(":refs/heads/{TEMPORARY}")
                    );
                }
            }
            assert_eq!(
                memory
                    .remote_refs
                    .contains_key(&format!("refs/heads/{TEMPORARY}")),
                failed
            );
            if failed {
                assert!(
                    memory
                        .stderr
                        .contains("cleanup can be retried independently")
                );
            }
            drop(memory);
            world.0.lock().unwrap().responses.remove(&deletion);
        }
        world.0.lock().unwrap().trace.clear();
        world.run(&flags).unwrap();
        assert!(!world.trace().iter().any(|event| event["args"][0] == "push"
            || matches!(event["method"].as_str(), Some("PATCH" | "DELETE"))
            || (event["method"] == "POST" && event["path"] != "graphql")));
    }
}
