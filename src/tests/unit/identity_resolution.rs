use super::*;
use crate::tests::unit::support::{World, commit};

fn inspect(
    world: &std::sync::Arc<World>,
    records: &Records,
    branches: &BTreeMap<String, String>,
    selected: &Commit,
) -> Result<Option<Binding>> {
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "",
    };
    resolve(&github, selected, records, branches, false, true)
}

#[test]
fn invalid_claims_are_rejected_in_every_reconciliation_role() {
    for (anchor, field) in [
        ("topic/1", "gitprhead"),
        ("topic/1", "gitprpullrequest"),
        ("topic/1", "gitprchangeid"),
        ("old", "gitprhead"),
        ("old", "gitprpullrequest"),
        ("old", "gitprchangeid"),
    ] {
        let world = World::new(1);
        {
            let mut memory = world.0.lock().unwrap();
            memory.jj = true;
            memory.config.insert(
                "branch.topic/1.gitPrHead".into(),
                "github.com/org/repo refs/heads/topic/1".into(),
            );
            memory
                .config
                .insert("branch.old.gitPrChangeId".into(), "change1".into());
        }
        let mut records = Records::decode("local\0file:/unit/repo/.git/config\0branch.topic/1.gitprhead\ngithub.com/org/repo refs/heads/topic/1\0local\0file:/unit/repo/.git/config\0branch.old.gitprchangeid\nchange1\0").unwrap();
        records.0.insert(
            format!("branch.{anchor}.{field}"),
            vec![crate::identity::store::Record {
                scope: "global".into(),
                origin: "file:global".into(),
                value: "invalid inherited claim".into(),
            }],
        );
        let branches = BTreeMap::from([("topic/1".into(), commit(1).hash)]);
        assert!(
            inspect(&world, &records, &branches, &commit(1)).is_err(),
            "{anchor}/{field}"
        );
    }
}

#[test]
fn resolver_rejects_ambiguous_anchors_and_invalid_identity_values() {
    let world = World::new(1);
    let branches = BTreeMap::from([("a".into(), commit(1).hash), ("b".into(), commit(1).hash)]);
    let mut c = commit(1);
    c.attrs.clear();
    assert!(inspect(&world, &Records::default(), &branches, &c).is_err());
    world.0.lock().unwrap().jj = true;
    c.change_id.clear();
    let branches = BTreeMap::from([("topic/1".into(), c.hash.clone())]);
    assert!(
        inspect(&world, &Records::default(), &branches, &c)
            .unwrap()
            .is_some()
    );
    for (field, value) in [
        ("gitprhead", "malformed"),
        ("gitprpullrequest", "invalid"),
        ("gitprchangeid", "two words"),
    ] {
        let records = Records::decode(&format!(
            "local\0file:/unit/repo/.git/config\0branch.topic/1.{field}\n{value}\0"
        ))
        .unwrap();
        assert!(inspect(&world, &records, &branches, &c).is_err());
    }
    let legacy = commit(1);
    let branches = BTreeMap::from([("topic/1".into(), commit(2).hash)]);
    let binding = inspect(&world, &Records::default(), &branches, &legacy)
        .unwrap()
        .unwrap();
    assert_eq!(binding.anchor.as_deref(), Some("topic/1"));
    assert_eq!(binding.anchor_oid.as_deref(), Some(commit(2).hash.as_str()));
}

#[test]
fn remote_review_evidence_must_have_a_valid_open_head_when_selected() {
    for (editor, invalid_head, closed) in [
        (false, true, false),
        (true, true, false),
        (true, false, true),
        (true, false, false),
    ] {
        let world = World::new(1);
        world.run(&["--no-stack"]).unwrap();
        {
            let mut memory = world.0.lock().unwrap();
            if invalid_head {
                memory.prs.get_mut(&101).unwrap()["head"]["ref"] = serde_json::json!("bad..head");
            }
            if closed {
                memory.prs.get_mut(&101).unwrap()["state"] = serde_json::json!("closed");
            }
            if !invalid_head && !closed {
                memory.prs.clear();
            }
        }
        let field = if editor {
            "github-pr-owner-number"
        } else {
            "gitprpullrequest"
        };
        let value = if editor {
            "org#repo#101"
        } else {
            "https://github.com/org/repo/pull/101"
        };
        let records = Records::decode(&format!(
            "local\0file:config\0branch.topic/1.{field}\n{value}\0"
        ))
        .unwrap();
        let mut c = commit(1);
        c.attrs.clear();
        let branches = BTreeMap::from([("topic/1".into(), c.hash.clone())]);
        assert!(inspect(&world, &records, &branches, &c).is_err());
    }
}

#[test]
fn fallback_discovery_validates_destinations_and_unselected_context() {
    let world = World::new(1);
    let config = world.config();
    let commands = world.commands();
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "",
    };
    assert!(
        discover(
            &github,
            &commit(1),
            "invalid..anchor",
            &Records::default(),
            true,
            true
        )
        .is_err()
    );
    assert_eq!(
        discover(
            &github,
            &commit(1),
            "topic/1",
            &Records::default(),
            true,
            false
        )
        .unwrap(),
        None
    );
    assert_eq!(
        discover(
            &github,
            &commit(1),
            "topic/1",
            &Records::default(),
            false,
            false
        )
        .unwrap(),
        None
    );
    world.reply(
        "git for-each-ref --format=%(push:remotename)%00%(push:remoteref) refs/heads/topic/1",
        0,
        "origin\0refs/heads/main",
    );
    assert!(
        discover(
            &github,
            &commit(1),
            "topic/1",
            &Records::default(),
            false,
            true
        )
        .is_err()
    );
}

#[test]
fn unselected_editor_and_push_evidence_requires_an_existing_review() {
    for present in [false, true] {
        let world = World::new(1);
        if present {
            world.run(&["--no-stack"]).unwrap();
        }
        let config = world.config();
        let commands = world.commands();
        let github = GitHub {
            session: crate::github::Session::default(),
            clock: None,
            config: &config,
            commands: &commands,
            transport: world.as_ref(),
            template: "",
        };
        world.reply(
            "git for-each-ref --format=%(push:remotename)%00%(push:remoteref) refs/heads/renamed",
            0,
            "origin\0refs/heads/topic/1",
        );
        assert_eq!(
            discover(
                &github,
                &commit(1),
                "renamed",
                &Records::default(),
                false,
                false
            )
            .unwrap()
            .is_some(),
            present
        );
        if present {
            let records = Records::decode(
                "local\0file:config\0branch.renamed.github-pr-owner-number\norg#repo#101\0",
            )
            .unwrap();
            let mut head = None;
            let mut review = None;
            verify_review(
                &github,
                Some("renamed"),
                &records,
                false,
                &mut head,
                &mut review,
            )
            .unwrap();
            assert_eq!(review, Some(101));
        }
        world.reply(
            "git for-each-ref --format=%(push:remotename)%00%(push:remoteref) refs/heads/renamed",
            0,
            "other\0refs/heads/topic/1",
        );
        assert!(
            discover(
                &github,
                &commit(1),
                "renamed",
                &Records::default(),
                false,
                true
            )
            .is_ok()
        );
    }
}

#[test]
fn unrelated_scope_conflicts_and_changed_lifecycle_are_not_absence() {
    let world = World::new(1);
    world.run(&["--no-stack"]).unwrap();
    let config = world.config();
    let commands = world.commands();
    world.0.lock().unwrap().prs.get_mut(&101).unwrap()["state"] = serde_json::json!("closed");
    let github = GitHub {
        session: crate::github::Session::default(),
        clock: None,
        config: &config,
        commands: &commands,
        transport: world.as_ref(),
        template: "",
    };
    github.observe(&["topic/1".into()]).unwrap();
    let mut c = commit(1);
    c.publication = Some(Binding {
        anchor: Some("topic/1".into()),
        anchor_oid: Some(c.hash.clone()),
        head: Head::new(&config.repo, "topic/1").unwrap(),
        review: Some(101),
        adopted: true,
        previous_anchor: None,
    });
    assert!(persist(&github, std::slice::from_mut(&mut c)).is_err());
    let records = Records::decode(
        "global\0file:global\0branch.other.gitprhead\ngithub.com/org/repo refs/heads/topic/1\0",
    )
    .unwrap();
    assert!(
        validate_unique(
            &[c],
            0,
            &records,
            &BTreeMap::from([("other".into(), commit(2).hash)])
        )
        .is_err()
    );
}
