//! Fresh observations and verified mutations for PR base transitions.

use anyhow::{Context, Result, bail};
use serde_json::json;

use super::{GitHub, guard, policy};
use crate::model::pull_request::RecoveryPullRequest;

impl GitHub<'_> {
    pub(crate) fn recovery_pr(&self, number: u64) -> Result<RecoveryPullRequest> {
        let pr = serde_json::from_slice(&self.request("GET", &format!("pulls/{number}"), None)?)
            .context("incomplete base recovery observation")?;
        policy::validate_recovery_observation(&pr, number, &self.config.repo.name)?;
        Ok(pr)
    }

    pub(crate) fn recovery_parent(&self, reference: &str) -> Result<RecoveryPullRequest> {
        let references = [reference.to_owned()];
        self.invalidate_heads(&references);
        self.observe(&references)?;
        let numbers = self
            .session
            .heads()
            .get(reference)
            .cloned()
            .unwrap_or_default();
        let [number] = numbers.as_slice() else {
            bail!(
                "obsolete base {reference:?} has missing or ambiguous PR history; refusing automatic base recovery"
            );
        };
        self.recovery_pr(*number)
    }

    pub(crate) fn retarget_recovery(
        &self,
        expected: &RecoveryPullRequest,
        base: &str,
    ) -> Result<()> {
        let current = self.recovery_pr(expected.number)?;
        policy::same_recovery_head(expected, &current, &expected.base.name)?;
        Self::require_recoverable_membership(&current)?;
        let effect = crate::model::outcome::Operation::Base(expected.number);
        self.commands.outcomes.start(effect.clone());
        let mutation = self.request_url_encoded(
            "PATCH",
            &self.repo_url(&format!("pulls/{}", expected.number)),
            Some(json!({"base":base}).to_string().as_bytes()),
            1,
            true,
        );
        self.invalidate_heads(std::slice::from_ref(&expected.head.name));
        // An unsuccessful response does not establish that the mutation failed.
        let observed = self
            .recovery_pr(expected.number)
            .and_then(|actual| policy::same_recovery_head(expected, &actual, base));
        if let Err(error) = observed {
            let detail = mutation.err().map_or_else(String::new, |cause| {
                format!("; original base update failure: {cause:#}")
            });
            return Err(error).with_context(|| format!("base recovery for PR #{} could not be confirmed; retain any completed PR changes and inspect before retrying{detail}", expected.number));
        }
        guard(&self.session.updated).insert(expected.number);
        self.commands.outcomes.confirm(effect);
        Ok(())
    }
}
