use super::*;

#[test]
fn later_failures_retain_confirmed_effects_and_rejections_resolve_uncertainty() {
    let outcomes = Outcomes::default();
    let operation = Operation::Labels(101);
    outcomes.start(operation.clone());
    let first = outcomes.snapshot()[&operation];
    assert!(!first.confirmed && first.unconfirmed);
    outcomes.confirm(operation.clone());
    outcomes.start(operation.clone());
    let partial = outcomes.snapshot()[&operation];
    assert!(partial.confirmed && partial.unconfirmed);
    outcomes.reject(operation.clone());
    let rejected = outcomes.snapshot()[&operation];
    assert!(rejected.confirmed && !rejected.unconfirmed);
    let creation = Operation::Creation("topic/2".into());
    outcomes.start(creation.clone());
    outcomes.reject(creation.clone());
    let rejected = outcomes.snapshot()[&creation];
    assert!(!rejected.confirmed && !rejected.unconfirmed);
}

#[test]
fn shared_outcome_evidence_survives_worker_unwinding() {
    let outcomes = Outcomes::default();
    let worker = outcomes.clone();
    assert!(
        std::thread::spawn(move || {
            let _guard = worker.0.lock().unwrap();
            panic!("fixture worker failure");
        })
        .join()
        .is_err()
    );
    outcomes.confirm(Operation::NativeStack);
    assert!(outcomes.snapshot()[&Operation::NativeStack].confirmed);
}
