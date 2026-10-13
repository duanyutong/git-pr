//! Deterministic expansion of configuration paths.

use std::path::PathBuf;

pub(crate) fn expand_path(path: &str, home: &str) -> PathBuf {
    PathBuf::from(
        path.strip_prefix('~')
            .map_or_else(|| path.to_owned(), |rest| format!("{home}{rest}")),
    )
}

/// Remove the command's record terminator whilst preserving significant path whitespace.
pub(super) fn command_path(output: &str) -> PathBuf {
    output.strip_suffix('\n').unwrap_or(output).into()
}
