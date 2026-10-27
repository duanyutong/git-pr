//! Publication records shared by identity resolution and commit metadata.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Head {
    pub repository: String,
    pub reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub anchor: Option<String>,
    pub anchor_oid: Option<String>,
    pub head: Head,
    /// A pinned review is distinct from the current review found by head name.
    pub review: Option<u64>,
    pub adopted: bool,
    pub previous_anchor: Option<String>,
}
