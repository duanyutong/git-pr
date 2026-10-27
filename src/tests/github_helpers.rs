//! Test support excluded from production coverage.

use std::time::Duration;

use anyhow::Result;

use super::{GitHub, guard};
use crate::model::Commit;

impl GitHub<'_> {
    pub(crate) fn expire_pr_observation_for_test(&self, number: u64) {
        guard(&self.session.pr_times).remove(&number);
    }

    pub(crate) fn expire_head_observation_for_test(&self, reference: &str) {
        let now = self.now();
        let expired = now.checked_sub(Duration::from_secs(31)).unwrap_or(now);
        guard(&self.session.head_times).insert(reference.into(), expired);
    }

    pub(crate) fn number(&self, commit: &mut Commit, base: &str) -> Result<u64> {
        if commit.skip && commit.remote_ref().is_empty() {
            return Ok(0);
        }
        let number = self.find(commit)?;
        if number != 0 || commit.skip {
            return Ok(number);
        }
        self.create(commit, base)?;
        Ok(commit.pr_number)
    }
}
