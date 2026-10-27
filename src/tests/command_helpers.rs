//! Test support excluded from production coverage.

use anyhow::Result;

use super::parallel_cached;

pub(crate) fn parallel<T: Send, R: Send>(
    items: Vec<T>,
    function: impl Fn(T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    parallel_cached(items, |_| None, function)
}
