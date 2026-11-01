use super::*;
use crate::{
    commands::{CommandOutput, CommandRunner, StructuredOutput},
    tests::unit::support::World,
};
use std::{
    ffi::OsString,
    path::Path,
    sync::{Arc, Mutex},
};

fn oid(value: u64) -> String {
    format!("{value:040x}")
}
fn graph(rows: &[(u64, &[u64])]) -> Parents {
    rows.iter()
        .map(|(node, parents)| {
            (
                oid(*node),
                Some(parents.iter().map(|value| oid(*value)).collect()),
            )
        })
        .collect()
}

#[test]
fn shared_boundaries_prove_answers_without_older_objects() {
    let mut parents = graph(&[(3, &[2]), (4, &[2]), (2, &[1])]);
    parents.insert(oid(1), None);
    assert_eq!(query(&oid(3), &oid(4), &parents).0, Some(false));
    assert_eq!(query(&oid(2), &oid(4), &parents).0, Some(true));
    assert_eq!(query(&oid(4), &oid(4), &Parents::new()).0, Some(true));
    assert_eq!(query(&oid(4), &oid(9), &parents).0, None);
    assert!(query(&oid(4), &oid(9), &parents).1.contains(&oid(9)));
    parents.insert(oid(9), None);
    assert_eq!(query(&oid(4), &oid(9), &parents), (None, BTreeSet::new()));
    assert_eq!(query(&oid(9), &oid(0), &graph(&[(0, &[])])).0, Some(false));
}

#[derive(Debug)]
struct Objects {
    parents: Parents,
    calls: Mutex<Vec<Vec<String>>>,
    status: i32,
    malformed: bool,
    history_reply: Option<Vec<u8>>,
}
impl CommandRunner for Objects {
    fn edit_messages(
        &self,
        _: &str,
        _: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
        _: &[(String, String)],
    ) -> Result<CommandOutput> {
        unreachable!()
    }

    fn execute(
        &self,
        _: &str,
        _: &[String],
        _: &Path,
        _: &BTreeMap<OsString, OsString>,
    ) -> Result<CommandOutput> {
        unreachable!()
    }
    fn execute_data(
        &self,
        program: &str,
        args: &[String],
        _: &Path,
        env: &BTreeMap<OsString, OsString>,
        input: &[u8],
    ) -> Result<StructuredOutput> {
        if self.status < 0 {
            anyhow::bail!("fixture process start failure");
        }
        assert_eq!(program, "git");
        let history = args.get(1).is_some_and(|arg| arg == "rev-list");
        if history {
            assert!(args.iter().any(|arg| arg == "--max-count=128"));
            assert!(args.iter().any(|arg| arg == "--format=raw"));
            assert!(args.iter().any(|arg| arg == "--missing=print"));
        } else {
            assert_eq!(args, ["--no-replace-objects", "cat-file", "--batch"]);
        }
        assert_eq!(
            env.get(&OsString::from("GIT_NO_LAZY_FETCH")),
            Some(&OsString::from("1"))
        );
        let nodes = std::str::from_utf8(input)?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(nodes.len() <= 128);
        self.calls.lock().unwrap().push(nodes.clone());
        let mut stdout = if history {
            crate::tests::unit::support::raw_parent_history(&nodes, |oid| {
                self.parents.get(oid).and_then(Option::as_ref)
            })
        } else {
            Vec::new()
        };
        for node in nodes.into_iter().filter(|_| !history) {
            if let Some(Some(parents)) = self.parents.get(&node) {
                let body = format!(
                    "tree {}\n{}\nnon-UTF8 message: ",
                    oid(0),
                    parents.iter().fold(String::new(), |mut text, parent| {
                        use std::fmt::Write as _;
                        writeln!(text, "parent {parent}").unwrap();
                        text
                    })
                );
                let mut body = body.into_bytes();
                body.push(0xff);
                stdout.extend_from_slice(format!("{node} commit {}\n", body.len()).as_bytes());
                stdout.extend(body);
                stdout.push(b'\n');
            } else {
                stdout.extend_from_slice(format!("{node} missing\n").as_bytes());
            }
        }
        if self.malformed {
            stdout = b"invalid\n".to_vec();
        }
        if history && let Some(reply) = &self.history_reply {
            stdout.clone_from(reply);
        }
        Ok(StructuredOutput {
            status: self.status,
            stdout,
            stderr: b"fixture error".to_vec(),
        })
    }
}
fn observe_objects(
    parents: Parents,
    status: i32,
    malformed: bool,
    pairs: &BTreeSet<(String, String)>,
) -> Result<BTreeMap<(String, String), bool>> {
    let world = World::new(0);
    let config = world.config();
    let mut commands = world.commands();
    commands.runner = Arc::new(Objects {
        parents,
        calls: Mutex::default(),
        status,
        malformed,
        history_reply: None,
    });
    observe(
        Git {
            commands: &commands,
            repo: &config.repo,
        },
        pairs,
    )
}

#[test]
fn shallow_stack_exclusions_stop_at_shared_history_and_refuse_unknown_base_paths() {
    let world = World::new(0);
    let config = world.config();
    for (rows, missing, base, tip, expected) in [
        (
            vec![(2, vec![1]), (3, vec![1])],
            vec![1],
            2,
            3,
            Some(vec![3]),
        ),
        (
            vec![(2, vec![1]), (3, vec![1]), (1, vec![0, 9])],
            vec![0, 9],
            2,
            3,
            Some(vec![3]),
        ),
        (
            vec![(2, vec![3]), (3, vec![1])],
            vec![1],
            2,
            3,
            Some(vec![]),
        ),
        (vec![(2, vec![1, 9]), (3, vec![1])], vec![1, 9], 2, 3, None),
        (vec![(2, vec![1]), (3, vec![1, 9])], vec![1, 9], 2, 3, None),
        (vec![(0, vec![]), (3, vec![1])], vec![1], 0, 3, None),
        (vec![(0, vec![]), (3, vec![])], vec![], 0, 3, Some(vec![3])),
    ] {
        let parents = rows
            .into_iter()
            .map(|(node, values)| (oid(node), Some(values.into_iter().map(oid).collect())))
            .chain(missing.into_iter().map(|node| (oid(node), None)))
            .collect();
        let mut commands = world.commands();
        commands.runner = Arc::new(Objects {
            parents,
            calls: Mutex::default(),
            status: 0,
            malformed: false,
            history_reply: None,
        });
        let result = stack(
            Git {
                commands: &commands,
                repo: &config.repo,
            },
            &oid(base),
            &oid(tip),
        );
        if let Some(expected) = expected {
            assert_eq!(
                result
                    .unwrap()
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                expected.into_iter().map(oid).collect::<Vec<_>>()
            );
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn shallow_stack_pages_validate_each_selected_commit_once() {
    use crate::tests::unit::support::LINEARITY_CHECKS;
    for depth in [10_u64, 100, 500, 1000] {
        let parents: Parents = std::iter::once((oid(0), Some(vec![])))
            .chain((1..=depth).map(|node| (oid(node), Some(vec![oid(node - 1)]))))
            .collect();
        let world = World::new(0);
        let config = world.config();
        let runner = Arc::new(Objects {
            parents,
            calls: Mutex::default(),
            status: 0,
            malformed: false,
            history_reply: None,
        });
        let mut commands = world.commands();
        commands.runner = runner.clone();
        LINEARITY_CHECKS.set(0);
        let selected = stack(
            Git {
                commands: &commands,
                repo: &config.repo,
            },
            &oid(0),
            &oid(depth),
        )
        .unwrap();
        assert_eq!(selected.len(), usize::try_from(depth).unwrap());
        assert_eq!(LINEARITY_CHECKS.get(), selected.len());
        assert!(
            runner.calls.lock().unwrap().len() <= 2 + usize::try_from(depth.div_ceil(128)).unwrap()
        );
    }
}

#[test]
fn shallow_stack_refuses_unavailable_proofs_and_malformed_resumption_pages() {
    for proof_available in [false, true] {
        let world = World::new(0);
        let config = world.config();
        let parents = if proof_available {
            graph(&[(0, &[]), (3, &[1]), (1, &[0])])
        } else {
            Parents::new()
        };
        let mut commands = world.commands();
        commands.runner = Arc::new(Objects {
            parents,
            calls: Mutex::default(),
            status: 0,
            malformed: false,
            history_reply: Some(b"invalid history\n".to_vec()),
        });
        assert!(
            stack(
                Git {
                    commands: &commands,
                    repo: &config.repo
                },
                &oid(0),
                &oid(3)
            )
            .is_err()
        );
    }
    assert!(super::super::snapshot::require_linear("short", &[oid(0), oid(1)]).is_err());
}
#[test]
fn batched_local_objects_resolve_merges_missing_history_and_failures() {
    let parents = graph(&[(3, &[1, 2]), (2, &[1]), (1, &[0]), (4, &[1])]);
    let pairs = BTreeSet::from([(oid(2), oid(3)), (oid(3), oid(4)), (oid(4), oid(3))]);
    let answers = observe_objects(parents.clone(), 0, false, &pairs).unwrap();
    assert!(answers[&(oid(2), oid(3))]);
    assert!(!answers[&(oid(3), oid(4))]);
    assert!(!answers[&(oid(4), oid(3))]);
    for (status, malformed) in [(-1, false), (1, false), (0, true)] {
        assert!(observe_objects(parents.clone(), status, malformed, &pairs).is_err());
    }
    let error = observe_objects(Parents::new(), 0, false, &pairs).unwrap_err();
    assert!(error.to_string().contains("unavailable commit"));
    assert_eq!(
        observe_objects(Parents::new(), 0, false, &BTreeSet::new()).unwrap(),
        BTreeMap::new()
    );
    let wide = (1..=260).map(|value| (oid(value), Some(vec![]))).collect();
    let pairs = (1..=260).map(|value| (oid(value), oid(999))).collect();
    assert!(observe_objects(wide, 0, false, &pairs).is_err());
}
#[test]
fn commit_framing_rejects_incomplete_or_unexpected_objects() {
    let requested = [oid(1)];
    for bytes in [
        b"".as_slice(),
        b"\xff\n",
        b"wrong missing\n",
        b"wrong commit 0\n\n",
    ] {
        assert!(decode(bytes, &requested).is_err());
    }
    for tail in [
        "commit invalid\n",
        "commit 1\n",
        "commit 0\nx",
        "commit 9\nparent x\n\n",
        "commit 10\nparent x\n\n\n",
    ] {
        assert!(decode(format!("{} {tail}", oid(1)).as_bytes(), &requested).is_err());
    }
    let body = format!("tree {}\nparent ", oid(0));
    let mut body = body.into_bytes();
    body.extend_from_slice(b"\xff\n\n");
    let mut bytes = format!("{} commit {}\n", oid(1), body.len()).into_bytes();
    bytes.extend_from_slice(&body);
    bytes.push(b'\n');
    assert!(decode(&bytes, &requested).is_err());
    for body in [
        format!("tree {}\nparent invalid\n\n", oid(0)),
        "tree invalid\n\n".into(),
        format!("tree {}\nunterminated", oid(0)),
    ] {
        assert!(
            decode(
                format!("{} commit {}\n{body}\n", oid(1), body.len()).as_bytes(),
                &requested
            )
            .is_err()
        );
    }
    let body = format!("tree {}\nparent {}\n\n", "a".repeat(64), "b".repeat(64));
    assert_eq!(
        decode(
            format!("{} commit {}\n{body}\n", oid(1), body.len()).as_bytes(),
            &requested
        )
        .unwrap()[&oid(1)],
        Some(vec!["b".repeat(64)])
    );
    assert!(decode(b"trailing", &[]).is_err());
}

#[test]
fn long_relevant_paths_use_bounded_history_pages() {
    for depth in [10_u64, 100, 500, 1000] {
        let mut parents = graph(&[(1, &[0])]);
        for n in 0..=depth + 1 {
            let id = if n == 0 { 0 } else { 1_000_000 + n };
            let next = if n == depth + 1 {
                vec![]
            } else {
                vec![oid(1_000_000 + n + 1)]
            };
            parents.insert(oid(id), Some(next));
        }
        for n in 1..=depth {
            parents.insert(
                oid(10_000 + n),
                Some(vec![oid(if n == 1 { 0 } else { 10_000 + n - 1 })]),
            );
        }
        let world = World::new(0);
        let config = world.config();
        let mut commands = world.commands();
        let objects = Arc::new(Objects {
            parents,
            calls: Mutex::default(),
            status: 0,
            malformed: false,
            history_reply: None,
        });
        commands.runner = objects.clone();
        let answer = observe(
            Git {
                commands: &commands,
                repo: &config.repo,
            },
            &BTreeSet::from([(oid(1), oid(10_000 + depth))]),
        )
        .unwrap();
        assert!(!answer[&(oid(1), oid(10_000 + depth))]);
        let calls = objects.calls.lock().unwrap();
        let command_count = calls.len();
        let bounded_requests = calls.iter().all(|roots| roots.len() <= 128);
        drop(calls);
        let bound = 4 * usize::try_from(depth).unwrap().div_ceil(127) + 5;
        assert!(
            command_count <= bound,
            "depth={depth}, calls={command_count}, bound={bound}"
        );
        assert!(bounded_requests);
        eprintln!("ancestry depth={depth}: {command_count} bounded command requests");
    }
}

#[test]
fn raw_history_framing_preserves_physical_parents_and_opaque_messages() {
    let record = |id, parent| {
        format!(
            "commit {}\ntree {}\nparent {}\nauthor Fixture <fixture@example.invalid> 1 +0000\ncommitter Fixture <fixture@example.invalid> 1 +0000\n\n    fixture\n\n",
            oid(id),
            oid(0),
            oid(parent)
        )
    };
    let mut bytes = record(2, 1).into_bytes();
    bytes.extend_from_slice(b"    \xff\n");
    bytes.extend_from_slice(format!("?{}\n", oid(9)).as_bytes());
    let actual = decode_history(&bytes).unwrap();
    assert_eq!(actual[&oid(2)], Some(vec![oid(1)]));
    assert_eq!(actual[&oid(9)], None);
    let custom = format!(
        "commit {}\ntree {}\ncommit {}\nparent {}\n\n    commit {}\n\n",
        oid(3),
        oid(0),
        oid(8),
        oid(2),
        oid(7)
    );
    let actual = decode_history(custom.as_bytes()).unwrap();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[&oid(3)], Some(vec![oid(2)]));
    let mut bad_tree = format!("commit {}\ntree ", oid(2)).into_bytes();
    bad_tree.extend_from_slice(b"\xff\n\n");
    let mut bad_parent = format!("commit {}\ntree {}\nparent ", oid(2), oid(0)).into_bytes();
    bad_parent.extend_from_slice(b"\xff\n\n");
    for value in [
        b"bad\n".to_vec(),
        b"commit \xff\n".to_vec(),
        b"commit invalid\n".to_vec(),
        format!("commit {}\nparent {}\n\n", oid(2), oid(1)).into_bytes(),
        format!("commit {}\ntree invalid\n\n", oid(2)).into_bytes(),
        format!("commit {}\ntree {}\ntree {}\n\n", oid(2), oid(0), oid(0)).into_bytes(),
        format!("commit {}\ntree {}\nparent invalid\n\n", oid(2), oid(0)).into_bytes(),
        format!("commit {}\ntree {}\n", oid(2), oid(0)).into_bytes(),
        format!("commit {}\nparent {}\n\n?{}\n", oid(2), oid(1), oid(9)).into_bytes(),
        format!(
            "commit {}\nparent {}\n\ncommit {}\ntree {}\n\n",
            oid(2),
            oid(1),
            oid(3),
            oid(0)
        )
        .into_bytes(),
        [record(2, 1), record(2, 1)].concat().into_bytes(),
        [record(2, 1), format!("?{}\n", oid(2))]
            .concat()
            .into_bytes(),
        format!("?{}\n?{}\n", oid(9), oid(9)).into_bytes(),
        b"?invalid\n".to_vec(),
        b"?\xff\n".to_vec(),
        bad_tree,
        bad_parent,
    ] {
        assert!(
            decode_history(&value).is_err(),
            "{}",
            String::from_utf8_lossy(&value)
        );
    }
    let wide = (1..=129).map(|id| record(id, 0)).collect::<String>();
    assert!(decode_history(wide.as_bytes()).is_err());
    assert_eq!(decode_history(b"").unwrap(), Parents::new());
}

#[test]
fn bounded_history_requires_its_confirmed_starting_identity() {
    for reply in [
        Vec::new(),
        b"bad\n".to_vec(),
        format!("commit {}\ntree {}\n\n", oid(99), oid(0)).into_bytes(),
    ] {
        let world = World::new(0);
        let config = world.config();
        let mut commands = world.commands();
        let objects = Arc::new(Objects {
            parents: graph(&[(1, &[0]), (4, &[3]), (3, &[2]), (2, &[0]), (0, &[])]),
            calls: Mutex::default(),
            status: 0,
            malformed: false,
            history_reply: Some(reply),
        });
        commands.runner = objects;
        assert!(
            observe(
                Git {
                    commands: &commands,
                    repo: &config.repo
                },
                &BTreeSet::from([(oid(1), oid(4))])
            )
            .is_err()
        );
    }
}

#[test]
fn wide_frontiers_and_overlapping_history_pages_make_progress() {
    for overlap in [false, true] {
        let mut parents = graph(&[(1, &[]), (0, &[])]);
        for n in 1..=260_u64 {
            parents.insert(oid(1000 + n), Some(vec![oid(2000 + n)]));
            let parent = if overlap {
                if n < 260 { 2000 + n + 1 } else { 0 }
            } else if n == 1 {
                2257
            } else if (257..260).contains(&n) {
                2000 + n + 1
            } else {
                0
            };
            parents.insert(oid(2000 + n), Some(vec![oid(parent)]));
        }
        let pairs = (1..=260).map(|n| (oid(1), oid(1000 + n))).collect();
        let answers = observe_objects(parents, 0, false, &pairs).unwrap();
        assert_eq!(answers.len(), 260);
        assert!(answers.values().all(|answer| !answer));
    }
}
