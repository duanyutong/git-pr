mod commit;
pub(crate) mod outcome;
pub(crate) mod pull_request;
mod repository;
pub(crate) mod trailers;

pub(crate) use commit::{Commit, REMOTE_REF, comma_list, dedup, shorten_title};
pub(crate) use pull_request::PullRequest;
pub(crate) use repository::Repository;
