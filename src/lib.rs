//! Submission of commit stacks as GitHub pull requests.

pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod config;
pub(crate) mod credentials;
pub(crate) mod diagnostics;
pub(crate) mod git;
pub(crate) mod github;
pub(crate) mod identity;
pub(crate) mod jj;
pub(crate) mod model;
pub(crate) mod stack;
pub(crate) mod submission;

pub use submission::run;

#[cfg(test)]
pub(crate) mod tests;
