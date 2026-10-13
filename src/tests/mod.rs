// Fixture tables keep the reference cases together for auditability.
#![allow(clippy::too_many_lines)]
mod adapters;
mod assurance;
mod backends;
mod differential;
mod regressions;
mod support;
pub(crate) mod unit;
mod workflows;

mod http_service;

mod legacy;
