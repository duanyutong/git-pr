// Unit tests use values and in-memory doubles; external boundaries belong in other suites.
mod assurance;
mod boundaries;
mod contracts;
mod coverage_branches;
mod diagnostics;
mod errors;
mod fuzz_regressions;
mod policy;
mod properties;
mod pure;
pub(crate) mod support;
mod workflows;

mod efficiency;
mod local_protocol;
mod mutations;

pub(crate) mod live_progress;
mod local_optimisation;

mod terminal_results;
