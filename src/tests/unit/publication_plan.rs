use super::*;
use proptest::{prelude::any, prop_assert, prop_assert_eq, proptest};

fn stack(order: &[u64], generation: u64) -> (Vec<Commit>, Vec<String>) {
    let commits = order
        .iter()
        .map(|number| {
            let mut head = crate::tests::unit::support::commit(*number);
            head.hash = format!("{}", generation + number);
            head
        })
        .collect::<Vec<_>>();
    let bases = crate::stack::selection::resolve_bases(&commits, 0, "main");
    (commits, bases)
}

fn facts(
    desired: &DesiredPublication<'_>,
    observed: &ObservedPublication<'_>,
    old: &[u64],
    new: &[u64],
) -> Ancestry {
    let mut parents = BTreeMap::new();
    for (generation, order) in [(100, old), (200, new)] {
        let mut previous = "0".to_owned();
        for number in order {
            let head = format!("{}", generation + number);
            parents.insert(head.clone(), previous);
            previous = head;
        }
    }
    Ancestry(
        Ancestry::requirements(desired, observed)
            .unwrap()
            .into_iter()
            .map(|(ancestor, descendant)| {
                let mut current = descendant.as_str();
                while current != ancestor {
                    let Some(parent) = parents.get(current) else {
                        break;
                    };
                    current = parent;
                }
                let answer = current == ancestor;
                ((ancestor, descendant), answer)
            })
            .collect(),
    )
}

fn refs() -> BTreeMap<String, Option<String>> {
    BTreeMap::from([
        ("main".into(), Some("0".into())),
        ("topic/1".into(), Some("101".into())),
        ("topic/2".into(), Some("102".into())),
        ("topic/3".into(), Some("103".into())),
    ])
}

fn prs(order: &[u64]) -> BTreeMap<usize, (u64, String)> {
    order
        .iter()
        .enumerate()
        .map(|(index, number)| {
            (
                index,
                (
                    100 + number,
                    if *number == 1 {
                        "main".into()
                    } else {
                        format!("topic/{}", number - 1)
                    },
                ),
            )
        })
        .collect()
}

#[test]
fn planning_selects_direct_retargeting_or_staging_from_verified_graph_facts() {
    for order in [&[1, 2, 3][..], &[2, 1, 3][..], &[1, 4, 2, 3][..]] {
        let (commits, bases) = stack(order, 200);
        let desired = DesiredPublication::new(&commits, 0, &bases, true);
        let tips = refs();
        let mut identities = prs(order);
        identities.retain(|_, (number, _)| *number != 104);
        if order == [1, 2, 3] {
            identities.get_mut(&1).unwrap().1 = "main".into();
        }
        let observed = ObservedPublication {
            refs: &tips,
            prs: identities,
            trunk: "main",
            anchor: "0",
        };
        let plan = PublicationPlan::derive(
            &desired,
            &observed,
            &facts(&desired, &observed, &[1, 2, 3], order),
        )
        .unwrap();
        assert_eq!(
            plan.strategy,
            if order == [1, 2, 3] {
                Strategy::Retarget
            } else {
                Strategy::Stage("git-pr/transition/101".into())
            }
        );
        assert_eq!(plan.changing, (0..order.len()).collect::<Vec<_>>());
        assert!(plan.link_native);
    }
}

#[test]
fn trunk_is_a_named_target_not_a_required_ancestor_or_live_ref_expectation() {
    let (commits, bases) = stack(&[2, 1, 3], 200);
    let desired = DesiredPublication::new(&commits, 0, &bases, true);
    for live in [None, Some("0"), Some("unfetched-remote-tip")] {
        let mut tips = refs();
        tips.remove("main");
        if let Some(live) = live {
            tips.insert("main".into(), Some(live.into()));
        }
        let observed = ObservedPublication {
            refs: &tips,
            prs: prs(&[2, 1, 3]),
            trunk: "main",
            // This snapshot is on a different branch from both development
            // histories. No rebase onto its latest commit is required.
            anchor: "unrelated-integration-commit",
        };
        let ancestry = facts(&desired, &observed, &[1, 2, 3], &[2, 1, 3]);
        assert!(
            !ancestry
                .0
                .contains_key(&(observed.anchor.into(), "202".into()))
        );
        let plan = PublicationPlan::derive(&desired, &observed, &ancestry).unwrap();
        assert_eq!(
            plan.strategy,
            Strategy::Stage("git-pr/transition/101".into())
        );
        assert_eq!(plan.changing, [0, 1, 2]);
        assert!(plan.link_native);
    }
}

#[test]
fn planning_refuses_missing_or_inconsistent_evidence() {
    let (commits, bases) = stack(&[2, 1, 3], 200);
    let desired = DesiredPublication::new(&commits, 0, &bases, false);
    for case in [
        "ancestry", "final", "staging", "identity", "head", "trunk", "base",
    ] {
        let mut tips = refs();
        let mut identities = prs(&[2, 1, 3]);
        if case == "head" {
            tips.remove("topic/1");
        }

        let mut changed = bases.clone();
        if case == "base" {
            changed[0] = "absent".into();
        }
        if case == "identity" {
            for (number, _) in identities.values_mut() {
                *number = 0;
            }
        }
        let desired = if case == "base" {
            DesiredPublication::new(&commits, 0, &changed, false)
        } else {
            DesiredPublication::new(&commits, 0, &bases, false)
        };
        let observed = ObservedPublication {
            refs: &tips,
            prs: identities,
            trunk: "main",
            anchor: if case == "trunk" { "" } else { "0" },
        };
        if ["head", "trunk", "base"].contains(&case) {
            assert!(
                Ancestry::requirements(&desired, &observed).is_err(),
                "{case}"
            );
            continue;
        }
        let mut ancestry = facts(&desired, &observed, &[1, 2, 3], &[2, 1, 3]);
        match case {
            "ancestry" => ancestry.0.clear(),
            "final" => {
                ancestry.0.insert(("202".into(), "201".into()), false);
            }
            "staging" => {
                ancestry.0.insert(("101".into(), "0".into()), true);
            }
            _ => {}
        }
        assert!(
            PublicationPlan::derive(&desired, &observed, &ancestry).is_err(),
            "{case}"
        );
    }
    let mut equal = commits.clone();
    equal[0].hash = "0".into();
    let equal = DesiredPublication::new(&equal, 0, &bases, false);
    let tips = refs();
    let observed = ObservedPublication {
        refs: &tips,
        prs: prs(&[2, 1, 3]),
        trunk: "main",
        anchor: "0",
    };
    assert!(Ancestry::default().check_final(&equal, &observed).is_err());
    assert!(
        Ancestry::default()
            .check_staging(
                &desired,
                &ObservedPublication {
                    refs: &BTreeMap::new(),
                    prs: BTreeMap::new(),
                    trunk: "main",
                    anchor: "",
                }
            )
            .is_err()
    );
}

#[test]
fn independent_publications_commute_and_preserve_unselected_refs() {
    let (first, first_bases) = stack(&[1], 200);
    let (second, second_bases) = stack(&[2], 200);
    let mut results = Vec::new();
    for sequence in [
        [(&first, &first_bases), (&second, &second_bases)],
        [(&second, &second_bases), (&first, &first_bases)],
    ] {
        let mut remote = refs();
        for (commits, bases) in sequence {
            let desired = DesiredPublication::new(commits, 0, bases, false);
            let observed = ObservedPublication {
                refs: &remote,
                prs: BTreeMap::new(),
                trunk: "main",
                anchor: "0",
            };
            let plan = PublicationPlan::derive(&desired, &observed, &Ancestry::default()).unwrap();
            for index in plan.changing {
                remote.insert(
                    commits[index].remote_ref().into(),
                    Some(commits[index].hash.clone()),
                );
            }
        }
        assert_eq!(remote["topic/3"], Some("103".into()));
        results.push(remote);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn execution_guards_require_complete_facts_and_a_safe_selected_strategy() {
    let order = [2, 1, 3];
    let (commits, bases) = stack(&order, 200);
    let desired = DesiredPublication::new(&commits, 0, &bases, false);
    let mut tips = refs();
    let identities = prs(&order);
    let observed = ObservedPublication {
        refs: &tips,
        prs: identities.clone(),
        trunk: "main",
        anchor: "0",
    };
    let ancestry = facts(&desired, &observed, &[1, 2, 3], &order);
    assert!(
        ancestry
            .verify_strategy(&Strategy::Retarget, &desired, &observed)
            .is_err()
    );
    assert!(
        ancestry
            .verify_strategy(
                &Strategy::Stage("git-pr/transition/101".into()),
                &desired,
                &observed
            )
            .is_ok()
    );
    assert!(
        ancestry
            .verify_strategy(&Strategy::Publish, &desired, &observed)
            .is_err()
    );
    let compatible = ObservedPublication {
        refs: &tips,
        prs: desired
            .heads
            .iter()
            .map(|head| (head.index, (101 + head.index as u64, head.base.to_owned())))
            .collect(),
        trunk: "main",
        anchor: "0",
    };
    assert!(
        ancestry
            .verify_strategy(&Strategy::Publish, &desired, &compatible)
            .is_ok()
    );
    assert!(
        Ancestry::default()
            .direct_is_safe(&desired, &observed)
            .is_err()
    );
    assert!(
        Ancestry::default()
            .check_staging(&desired, &observed)
            .is_err()
    );
    let mut partial = Ancestry::default();
    for head in &desired.heads {
        partial.0.insert(
            (desired.tip(&observed, head.base).unwrap(), head.hash.into()),
            true,
        );
    }
    assert!(
        partial
            .verify_strategy(&Strategy::Retarget, &desired, &observed)
            .is_err()
    );
    assert!(PublicationPlan::derive(&desired, &observed, &partial).is_err());
    assert!(
        Ancestry::default()
            .verify_strategy(&Strategy::Publish, &desired, &observed)
            .is_err()
    );
    tips.remove("topic/2");
    let observed = ObservedPublication {
        refs: &tips,
        prs: identities,
        trunk: "main",
        anchor: "0",
    };
    assert!(ancestry.direct_is_safe(&desired, &observed).is_err());
    assert!(ancestry.check_staging(&desired, &observed).is_err());
    let (commits, mut bases) = stack(&[1], 200);
    bases[0] = "missing".into();
    let desired = DesiredPublication::new(&commits, 0, &bases, false);
    assert!(ancestry.check_final(&desired, &observed).is_err());
}

#[test]
fn temporary_names_require_an_unambiguous_nonzero_pr_identity() {
    for reference in [
        "topic/1",
        "git-pr/transition/invalid",
        "git-pr/transition/0",
        "git-pr/transition/001",
        "git-pr/transition/+1",
    ] {
        assert_eq!(temporary_owner(reference), None);
    }
    assert_eq!(temporary_owner("git-pr/transition/101"), Some(101));
    assert_eq!(temporary_base([0, 0].into_iter()), None);
}

proptest! {
    #[test]
    fn correct_publications_have_no_ref_mutations_and_ignore_unrelated_state(count in 1usize..20, seed in any::<u64>(), link_native in any::<bool>()) {
        let order = (1..=count as u64).collect::<Vec<_>>();
        let (commits, bases) = stack(&order, 200);
        let desired = DesiredPublication::new(&commits, 0, &bases, link_native);
        let mut remote = desired.heads.iter().map(|head| (head.reference.into(), Some(head.hash.into()))).collect::<BTreeMap<_, _>>();
        let observed = ObservedPublication { refs: &remote, prs: prs(&order), trunk: "main", anchor: "0" };
        let initial = PublicationPlan::derive(&desired, &observed, &Ancestry::default()).unwrap();
        prop_assert!(initial.changing.is_empty());
        prop_assert_eq!(&initial.strategy, &Strategy::Publish);
        prop_assert_eq!(initial.link_native, link_native);
        remote.insert(format!("unselected/{seed}"), Some(seed.to_string()));
        remote.insert("git-pr/transition/101".into(), Some("0".into()));
        let observed = ObservedPublication { refs: &remote, prs: prs(&order), trunk: "main", anchor: "0" };
        prop_assert_eq!(PublicationPlan::derive(&desired, &observed, &Ancestry::default()).unwrap(), initial);
    }
}
