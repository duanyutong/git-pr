//! Behavioural assertions for previously unobserved decision outcomes.

use crate::{
    cli::Flags,
    git::Git,
    github::{GitHub, Session},
    model::Repository,
};

use super::support::{World, commit};

#[test]
fn flag_prefixes_preserve_positionals_and_reject_empty_flag_keys() {
    for (arguments, expected) in [
        (vec!["-"], vec!["-"]),
        (vec!["--", "--unknown"], vec!["--unknown"]),
        (vec!["--", ""], vec![""]),
    ] {
        let flags = Flags::parse(arguments.into_iter().map(str::to_owned).collect()).unwrap();
        assert_eq!(flags.args, expected);
    }
    for invalid in ["--=value", "-=value", "---", "---draft"] {
        let error = Flags::parse(vec![invalid.into()]).unwrap_err();
        assert!(format!("{error:?}").contains("bad flag syntax"));
    }
}

#[test]
fn cached_all_state_lookup_retains_closed_identity_without_another_request() {
    let world = World::new(1);
    world.run(&["--no-stack"]).unwrap();
    world.0.lock().unwrap().prs.get_mut(&101).unwrap()["state"] = serde_json::json!("closed");
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        config: &config,
        session: Session::default(),
        commands: &commands,
        transport: world.as_ref(),
        clock: Some(world.as_ref()),
        template: "",
    };
    assert_eq!(github.find_head("topic/1", "all").unwrap(), 101);
    world.0.lock().unwrap().trace.clear();
    assert_eq!(github.find_head("topic/1", "all").unwrap(), 101);
    assert_eq!(github.cached_find(&commit(1)), Some(0));
    assert_eq!(world.trace(), Vec::<serde_json::Value>::new());
}

#[test]
fn an_empty_bookmark_change_identity_requires_ancestry_before_any_move() {
    let world = World::new(2);
    let old = "a".repeat(40);
    {
        let mut memory = world.0.lock().unwrap();
        memory.commits[0].hash.clone_from(&old);
        memory.commits[0].change_id.clear();
    }
    let published = commit(1);
    world.reply(
        &format!("git merge-base --is-ancestor {old} {}", published.hash),
        1,
        "",
    );
    let commands = world.commands();
    let repo = Repository {
        jj: true,
        ..Repository::default()
    };
    Git {
        commands: &commands,
        repo: &repo,
    }
    .sync_refs(&[published]);
    let trace = world.trace();
    assert!(trace.iter().any(|event| event["args"][0] == "merge-base"));
    assert!(!trace.iter().any(|event| {
        event["program"] == "jj"
            && (event["args"][0] == "bookmark"
                || (event["args"][0] == "git" && event["args"][1] == "export"))
    }));
    let memory = world.0.lock().unwrap();
    assert_eq!(memory.commits[0].hash, old);
    assert!(memory.stderr.contains("different change"));
    drop(memory);
}
