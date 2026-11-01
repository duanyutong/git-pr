//! Scoped ancestry from stored commit parents, independent of shallow grafts.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};

use super::Git;

type Parents = BTreeMap<String, Option<Vec<String>>>;

#[derive(Debug)]
struct Query {
    ancestor: String,
    ancestors: BTreeSet<String>,
    ancestor_pending: Vec<String>,
    ancestor_waiting: BTreeSet<String>,
    descendants: BTreeSet<String>,
    descendant_pending: Vec<String>,
    descendant_waiting: BTreeSet<String>,
    unavailable: BTreeSet<String>,
    answer: Option<bool>,
}

impl Query {
    fn new(ancestor: &str, descendant: &str) -> Self {
        Self {
            ancestor: ancestor.into(),
            ancestors: BTreeSet::from([ancestor.into()]),
            ancestor_pending: vec![ancestor.into()],
            ancestor_waiting: BTreeSet::new(),
            descendants: BTreeSet::new(),
            descendant_pending: vec![descendant.into()],
            descendant_waiting: BTreeSet::new(),
            unavailable: BTreeSet::new(),
            answer: None,
        }
    }

    fn advance(&mut self, parents: &Parents) {
        if self.answer.is_some() {
            return;
        }
        self.ancestor_pending
            .extend(std::mem::take(&mut self.ancestor_waiting));
        self.descendant_pending
            .extend(std::mem::take(&mut self.descendant_waiting));
        while let Some(oid) = self.ancestor_pending.pop() {
            match parents.get(&oid) {
                Some(Some(values)) => {
                    for parent in values {
                        if self.ancestors.insert(parent.clone()) {
                            self.unavailable.remove(parent);
                            self.ancestor_pending.push(parent.clone());
                        }
                    }
                }
                Some(None) => {}
                None => {
                    self.ancestor_waiting.insert(oid);
                }
            }
        }
        while let Some(oid) = self.descendant_pending.pop() {
            if oid == self.ancestor {
                self.answer = Some(true);
                return;
            }
            if self.ancestors.contains(&oid) || self.descendants.contains(&oid) {
                continue;
            }
            match parents.get(&oid) {
                Some(Some(values)) => {
                    self.descendants.insert(oid);
                    self.descendant_pending.extend(values.iter().cloned());
                }
                Some(None) => {
                    self.descendants.insert(oid.clone());
                    self.unavailable.insert(oid);
                }
                None => {
                    self.descendant_waiting.insert(oid);
                }
            }
        }
        if self.descendant_waiting.is_empty() && self.unavailable.is_empty() {
            self.answer = Some(false);
        }
    }

    fn needed(&self) -> impl Iterator<Item = &String> {
        self.descendant_waiting.iter().chain(&self.ancestor_waiting)
    }
}

pub(super) fn observe(
    git: Git<'_>,
    pairs: &BTreeSet<(String, String)>,
) -> Result<BTreeMap<(String, String), bool>> {
    let mut queries: Vec<_> = pairs.iter().map(|(a, d)| Query::new(a, d)).collect();
    acquire(git, &mut queries)?;
    Ok(pairs
        .iter()
        .cloned()
        .zip(queries.iter().filter_map(|query| query.answer))
        .collect())
}

/// Exclude proven base history before validating the selected linear path.
pub(super) fn stack(git: Git<'_>, base: &str, tip: &str) -> Result<Vec<(String, Vec<String>)>> {
    let mut query = Query::new(tip, base);
    let mut parents = acquire(git, std::slice::from_mut(&mut query))?;
    if query.answer == Some(true) {
        return Ok(Vec::new());
    }
    // A conclusive negative proof has visited every base path, stopping only
    // at known tip ancestors. Those visited nodes and edges belong to the base.
    let excluded: BTreeSet<_> = query
        .descendants
        .iter()
        .cloned()
        .chain(query.descendants.iter().flat_map(|oid| {
            parents
                .get(oid)
                .and_then(Option::as_ref)
                .into_iter()
                .flatten()
                .cloned()
        }))
        .chain([base.to_owned()])
        .collect();
    let mut selected = Vec::new();
    let mut current = tip.to_owned();
    while !excluded.contains(&current) {
        if !parents.contains_key(&current) {
            let page = read(git, std::slice::from_ref(&current), true).with_context(|| {
                format!("required stack history at {current} is unavailable locally")
            })?;
            parents.extend(page);
        }
        let values = parents
            .get(&current)
            .and_then(Option::as_ref)
            .with_context(|| format!("required stack commit {current} is unavailable locally"))?;
        super::snapshot::require_linear(&current, values)?;
        selected.push((current.clone(), values.clone()));
        let Some(parent) = values.first() else { break };
        current.clone_from(parent);
    }
    Ok(selected)
}

fn acquire(git: Git<'_>, queries: &mut [Query]) -> Result<Parents> {
    let mut parents = Parents::new();
    let mut initial = true;
    let mut ancestor_page = true;
    loop {
        for query in queries.iter_mut() {
            query.advance(&parents);
        }
        if queries.iter().all(|query| query.answer.is_some()) {
            return Ok(parents);
        }
        let needed: BTreeSet<_> = queries
            .iter()
            .filter(|query| query.answer.is_none())
            .flat_map(Query::needed)
            .cloned()
            .collect();
        if needed.is_empty() {
            let missing = parents
                .iter()
                .filter(|(_, value)| value.is_none())
                .map(|(oid, _)| oid.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "cannot establish required publication ancestry from local objects; unavailable commit(s): {missing}; synchronise the relevant history before retrying"
            );
        }
        let needed: Vec<_> = needed.into_iter().collect();
        for chunk in needed.chunks(128) {
            let chunk: Vec<_> = chunk
                .iter()
                .filter(|oid| !parents.contains_key(*oid))
                .cloned()
                .collect();
            if chunk.is_empty() {
                continue;
            }
            let next = queries
                .iter()
                .enumerate()
                .filter(|(_, query)| query.answer.is_none())
                .find_map(|(index, query)| {
                    let first = if ancestor_page {
                        &query.ancestor_waiting
                    } else {
                        &query.descendant_waiting
                    };
                    let second = if ancestor_page {
                        &query.descendant_waiting
                    } else {
                        &query.ancestor_waiting
                    };
                    first
                        .iter()
                        .chain(second)
                        .find(|oid| chunk.contains(*oid))
                        .map(|oid| (index, oid.clone()))
                });
            // Stored frontier edges can complete a proof without speculative
            // history acquisition, including where a shallow marker stops Git.
            parents.extend(read(git, &chunk, false)?);
            if !initial && let Some((index, root)) = next {
                for query in queries.iter_mut() {
                    query.advance(&parents);
                }
                if queries[index].answer.is_none()
                    && parents.get(&root).is_some_and(Option::is_some)
                {
                    let observed = read(git, &[root], true)?;
                    if !observed.keys().any(|oid| !parents.contains_key(oid)) {
                        // A page can stop at a shallow root already confirmed above.
                        // Its true parent remains an unresolved frontier for the next read.
                        continue;
                    }
                    for (oid, value) in observed {
                        parents.entry(oid).or_insert(value);
                    }
                }
            }
        }
        initial = false;
        ancestor_page = !ancestor_page;
    }
}

fn read(git: Git<'_>, requested: &[String], history: bool) -> Result<Parents> {
    let input = format!("{}\n", requested.join("\n"));
    let mut commands = git.commands.clone();
    commands.env.insert("GIT_NO_LAZY_FETCH".into(), "1".into());
    let args = if history {
        vec![
            "--no-replace-objects",
            "rev-list",
            "--max-count=128",
            "--format=raw",
            "--encoding=none",
            "--no-color",
            "--no-show-signature",
            "--missing=print",
            "--stdin",
        ]
    } else {
        vec!["--no-replace-objects", "cat-file", "--batch"]
    }
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let output = commands.runner.execute_data(
        "git",
        &args,
        &commands.cwd,
        &commands.env,
        input.as_bytes(),
    )?;
    if output.status != 0 {
        bail!(
            "could not read local commit parents: Git exited with {}; {}",
            output.status,
            crate::diagnostics::errors::redact_url_credentials(&String::from_utf8_lossy(
                &output.stderr
            ))
        );
    }
    if history {
        let observed = decode_history(&output.stdout)?;
        if !requested.iter().all(|oid| observed.contains_key(oid)) {
            bail!("incomplete bounded Git history observation");
        }
        Ok(observed)
    } else {
        decode(&output.stdout, requested)
    }
}

fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug)]
struct Record {
    oid: String,
    parents: Vec<String>,
    tree: bool,
    headers: bool,
}

fn store_record(parents: &mut Parents, record: Option<Record>) -> Result<()> {
    if let Some(record) = record {
        if record.headers {
            bail!("incomplete stored commit headers");
        }
        if !record.tree {
            bail!("missing stored tree identity");
        }
        if parents.insert(record.oid, Some(record.parents)).is_some() {
            bail!("duplicate Git history identity");
        }
    }
    Ok(())
}

fn decode_history(bytes: &[u8]) -> Result<Parents> {
    let mut parents = Parents::new();
    let mut current: Option<Record> = None;
    for raw in bytes.split_inclusive(|&byte| byte == b'\n') {
        let line = raw.strip_suffix(b"\n").unwrap_or(raw);
        if let Some(record) = current.as_mut().filter(|record| record.headers) {
            if line.is_empty() {
                record.headers = false;
            } else if let Some(oid) = line.strip_prefix(b"tree ") {
                let oid = std::str::from_utf8(oid)?;
                if record.tree || !valid_oid(oid) {
                    bail!("invalid stored tree identity");
                }
                record.tree = true;
            } else if let Some(oid) = line.strip_prefix(b"parent ") {
                let oid = std::str::from_utf8(oid)?;
                if !valid_oid(oid) {
                    bail!("invalid stored parent identity");
                }
                record.parents.push(oid.into());
            }
        } else if let Some(oid) = line.strip_prefix(b"commit ") {
            store_record(&mut parents, current.take())?;
            let oid = std::str::from_utf8(oid)?;
            if !valid_oid(oid) {
                bail!("invalid Git history identity");
            }
            current = Some(Record {
                oid: oid.into(),
                parents: Vec::new(),
                tree: false,
                headers: true,
            });
        } else if let Some(oid) = line.strip_prefix(b"?") {
            store_record(&mut parents, current.take())?;
            let oid = std::str::from_utf8(oid)?;
            if !valid_oid(oid) {
                bail!("invalid missing Git history identity");
            }
            if parents.insert(oid.into(), None).is_some() {
                bail!("duplicate Git history identity");
            }
        } else if !line.is_empty() && !line.starts_with(b"    ") {
            bail!("invalid raw Git history framing");
        }
    }
    store_record(&mut parents, current)?;
    if parents.values().filter(|value| value.is_some()).count() > 128 {
        bail!("Git history observation exceeded its commit bound");
    }
    Ok(parents)
}

#[cfg(test)]
fn query(ancestor: &str, descendant: &str, parents: &Parents) -> (Option<bool>, BTreeSet<String>) {
    let mut query = Query::new(ancestor, descendant);
    query.advance(parents);
    (
        query.answer,
        if query.answer.is_some() {
            BTreeSet::new()
        } else {
            query.needed().cloned().collect()
        },
    )
}

fn decode(mut bytes: &[u8], requested: &[String]) -> Result<Parents> {
    let mut parents = Parents::new();
    for expected in requested {
        let end = bytes
            .iter()
            .position(|&byte| byte == b'\n')
            .context("missing Git object header")?;
        let header = std::str::from_utf8(&bytes[..end])?;
        bytes = &bytes[end + 1..];
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields == [expected.as_str(), "missing"] {
            parents.insert(expected.clone(), None);
            continue;
        }
        let [actual, "commit", size] = fields.as_slice() else {
            bail!("invalid Git commit header");
        };
        if *actual != expected {
            bail!("unexpected Git commit identity");
        }
        let size = size.parse::<usize>()?;
        let body = bytes.get(..size).context("incomplete Git commit body")?;
        if bytes.get(size) != Some(&b'\n') {
            bail!("missing Git commit terminator");
        }
        bytes = &bytes[size + 1..];
        let tree = body
            .split(|&byte| byte == b'\n')
            .next()
            .and_then(|line| line.strip_prefix(b"tree "))
            .context("missing stored tree identity")?;
        if !matches!(tree.len(), 40 | 64)
            || !tree.iter().all(u8::is_ascii_hexdigit)
            || !body.windows(2).any(|bytes| bytes == b"\n\n")
        {
            bail!("invalid stored commit headers");
        }
        let mut values = Vec::new();
        for line in body
            .split(|&byte| byte == b'\n')
            .take_while(|line| !line.is_empty())
        {
            if let Some(oid) = line.strip_prefix(b"parent ") {
                let oid = std::str::from_utf8(oid)?;
                if !matches!(oid.len(), 40 | 64)
                    || !oid.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    bail!("invalid stored parent identity");
                }
                values.push(oid.to_owned());
            }
        }
        parents.insert(expected.clone(), Some(values));
    }
    if !bytes.is_empty() {
        bail!("unexpected trailing Git objects");
    }
    Ok(parents)
}

#[cfg(test)]
#[path = "../tests/unit/ancestry.rs"]
mod unit;
