// Integration fixture tables remain grouped for auditability.
#![allow(clippy::too_many_lines)]
mod adapters;
mod assurance;
mod backends;
mod identity;
mod regressions;
mod support;
pub(crate) mod unit;
mod workflows;

mod http_service;

pub(crate) mod model_helpers;
