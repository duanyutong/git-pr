//! Repository metadata shared by discovery, policy and adapters.

use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub(crate) struct Repository {
    pub root: PathBuf,
    pub remote: String,
    pub host: String,
    pub name: String,
    pub trunk: String,
    pub user: String,
    pub email: String,
    pub jj: bool,
    pub branchless: bool,
}
