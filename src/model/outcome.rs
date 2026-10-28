//! Invocation-scoped evidence of attempted and confirmed application effects.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Operation {
    Branch(String),
    Creation(String),
    Base(u64),
    Description(u64),
    Labels(u64),
    NativeStack,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Effect {
    pub confirmed: bool,
    pub unconfirmed: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Outcomes(Arc<Mutex<BTreeMap<Operation, Effect>>>);

impl Outcomes {
    fn effects(&self) -> MutexGuard<'_, BTreeMap<Operation, Effect>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn start(&self, operation: Operation) {
        self.effects().entry(operation).or_default().unconfirmed = true;
    }

    pub(crate) fn confirm(&self, operation: Operation) {
        self.effects().insert(
            operation,
            Effect {
                confirmed: true,
                unconfirmed: false,
            },
        );
    }

    pub(crate) fn reject(&self, operation: Operation) {
        self.effects().entry(operation).or_default().unconfirmed = false;
    }

    pub(crate) fn snapshot(&self) -> BTreeMap<Operation, Effect> {
        self.effects().clone()
    }
}

#[cfg(test)]
#[path = "../tests/unit/outcomes.rs"]
mod unit;
