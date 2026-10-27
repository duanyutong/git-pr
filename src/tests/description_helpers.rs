//! Test adapters excluded from production coverage.

use std::{collections::HashSet, sync::Arc};

use anyhow::Result;

use super::{Body, BodyCache, Footer, HistoryEntry, StackRenderer};
use crate::model::Commit;

impl Body {
    pub(crate) fn parse(source: &str) -> Result<Self> {
        Self::analyse(source.into())
    }

    pub(crate) fn history(&self) -> Vec<HistoryEntry> {
        let mut history = Vec::new();
        self.append_history(&mut history, &mut HashSet::new());
        history
    }

    pub(crate) fn render_update(
        &self,
        message: &str,
        stack_info: &str,
        template: &str,
    ) -> Result<Option<String>> {
        self.render_footer(message, &Footer::borrowed(stack_info), template)
    }

    pub(crate) fn render(&self, message: &str, stack_info: &str, template: &str) -> Result<String> {
        Ok(self
            .render_update(message, stack_info, template)?
            .unwrap_or_else(|| self.text().to_owned()))
    }
}

impl Footer<'_> {
    fn borrowed(text: &str) -> Footer<'_> {
        Footer {
            before: text,
            ..Footer::default()
        }
    }
}

impl BodyCache {
    pub(crate) fn get(&mut self, number: u64, source: &str) -> Result<&Body> {
        self.get_shared(number, &Arc::from(source))
    }
}

impl StackRenderer<'_> {
    pub(crate) fn render(&self, current: &Commit) -> String {
        self.footer(current).parts().concat()
    }
}

pub(crate) fn extract_history(body: &str) -> Vec<HistoryEntry> {
    Body::parse(body).unwrap().history()
}

pub(crate) fn generate_body(
    message: &str,
    existing: &str,
    stack_info: &str,
    template: &str,
) -> String {
    Body::parse(existing)
        .unwrap()
        .render(message, stack_info, template)
        .unwrap()
}
