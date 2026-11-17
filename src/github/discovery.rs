//! Invocation-scoped branch tips and negative commit evidence, independent of PR content.

use std::{collections::BTreeMap, fmt::Write as _, time::Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::{GitHub, guard};

type Tips = BTreeMap<String, Option<String>>;

#[derive(Debug, Default)]
pub(super) struct Discovery {
    active: bool,
    disabled: bool,
    pending: Vec<String>,
    tips: BTreeMap<String, (Instant, Option<String>)>,
}

#[derive(Debug)]
pub(super) struct Probe {
    references: Vec<String>,
    objects: Vec<String>,
}

impl GitHub<'_> {
    pub(crate) fn prepare_discovery(&self, hashes: Vec<String>) {
        let mut discovery = guard(&self.session.discovery);
        discovery.active = self.config.repo.host.eq_ignore_ascii_case("github.com");
        discovery.pending = hashes;
    }

    pub(super) fn discovery_probe(&self, references: Vec<String>) -> Option<Probe> {
        let discovery = guard(&self.session.discovery);
        (discovery.active && !discovery.disabled).then(|| Probe {
            references,
            objects: discovery.pending.iter().take(25).cloned().collect(),
        })
    }

    pub(super) fn disable_discovery(&self) {
        let mut discovery = guard(&self.session.discovery);
        discovery.disabled = true;
        discovery.tips.clear();
        discovery.pending.clear();
        drop(discovery);
        self.commands
            .explain("Publication discovery unavailable; checking remote branches through Git.");
    }

    pub(crate) fn publication_refs(&self, references: &[String]) -> Result<Option<Tips>> {
        let missing = {
            let discovery = guard(&self.session.discovery);
            if !discovery.active || discovery.disabled {
                return Ok(None);
            }
            let missing = references
                .iter()
                .filter(|reference| {
                    !discovery
                        .tips
                        .get(*reference)
                        .is_some_and(|(time, _)| self.fresh(time))
                })
                .cloned()
                .collect::<Vec<_>>();
            drop(discovery);
            missing
        };
        self.commands.explain(&format!(
            "Publication ref cache: {} requested, {} fresh, {} require acquisition.",
            references.len(),
            references.len() - missing.len(),
            missing.len()
        ));
        let selected = |reference: &&String| !reference.starts_with("git-pr/transition/");
        let total = references.iter().filter(selected).count();
        let mut progress = (total != 0 && !self.commands.terminal.has_progress())
            .then(|| self.commands.progress("Remote branches checked", total));
        if let Some(progress) = &mut progress {
            progress.advance(total - missing.iter().filter(selected).count());
        }
        for chunk in missing.chunks(25) {
            let probe = Probe {
                references: chunk.to_vec(),
                objects: Vec::new(),
            };
            let mut declarations = String::from("$owner:String!,$name:String!");
            let mut fields = String::new();
            let (owner, name) = self
                .config
                .repo
                .name
                .split_once('/')
                .context("invalid repository identity")?;
            let mut variables = json!({"owner":owner,"name":name});
            probe.append(self, &mut declarations, &mut fields, &mut variables);
            let response = self.request_url("POST", &format!("https://api.{}/graphql",self.config.repo.host), Some(&json!({
                "query":format!("query PublicationRefs({declarations}){{repository(owner:$owner,name:$name){{{fields}}}}}"),"variables":variables
            })))?;
            let value: Value = serde_json::from_slice(&response.body)
                .context("invalid publication ref response")?;
            if probe.unsupported(&value) {
                self.disable_discovery();
                return Ok(None);
            }
            probe.remember(self, &value, response.acquired)?;
            if guard(&self.session.discovery).disabled {
                return Ok(None);
            }
            if let Some(progress) = &mut progress {
                progress.advance(chunk.iter().filter(selected).count());
            }
        }
        let now = self.now();
        let discovery = guard(&self.session.discovery);
        let tips = discovery.verified_tips(references, now)?;
        let oldest = references
            .iter()
            .filter_map(|reference| discovery.tips.get(reference))
            .map(|(time, _)| now.saturating_duration_since(*time))
            .max()
            .unwrap_or_default();
        drop(discovery);
        self.commands.explain(&format!(
            "Publication ref evidence accepted: {} refs; oldest acquisition {:.3}s ago.",
            tips.len(),
            oldest.as_secs_f64()
        ));
        Ok(Some(tips))
    }
}

impl Discovery {
    fn verified_tips(&self, references: &[String], now: Instant) -> Result<Tips> {
        references
            .iter()
            .map(|reference| {
                let (time, tip) = self
                    .tips
                    .get(reference)
                    .context("missing publication ref evidence")?;
                if now.saturating_duration_since(*time) > super::limits::FRESHNESS {
                    bail!("publication ref evidence expired during acquisition");
                }
                Ok((reference.clone(), tip.clone()))
            })
            .collect()
    }
}

impl Probe {
    pub(super) fn append(
        &self,
        github: &GitHub<'_>,
        declarations: &mut String,
        fields: &mut String,
        variables: &mut Value,
    ) {
        fields.push_str("nameWithOwner ");
        for (index, reference) in self.references.iter().enumerate() {
            let _ = write!(declarations, ",$publicationRef{index}:String!");
            let _ = write!(
                fields,
                "publicationRef{index}:ref(qualifiedName:$publicationRef{index}){{name prefix target{{__typename oid}}}} "
            );
            variables[format!("publicationRef{index}")] = json!(format!("refs/heads/{reference}"));
        }
        if !self.objects.is_empty() {
            declarations.push_str(",$publicationControl:String!");
            variables["publicationControl"] =
                json!(format!("refs/heads/{}", github.config.repo.trunk));
            fields.push_str(
                "publicationControl:object(expression:$publicationControl){__typename oid} ",
            );
            for (index, hash) in self.objects.iter().enumerate() {
                let _ = write!(declarations, ",$publicationObject{index}:GitObjectID!");
                let _ = write!(
                    fields,
                    "publicationObject{index}:object(oid:$publicationObject{index}){{__typename oid}} "
                );
                variables[format!("publicationObject{index}")] = json!(hash);
            }
        }
    }

    pub(super) fn unsupported(&self, value: &Value) -> bool {
        value
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| {
                !errors.is_empty()
                    && errors.iter().all(|error| {
                        let path = error["path"].to_string();
                        let message = error["message"].as_str().unwrap_or_default();
                        (path.contains("publication") && error["type"] == "FORBIDDEN")
                            || (message.contains("Cannot query field")
                                && ((!self.references.is_empty() && message.contains("ref"))
                                    || (!self.objects.is_empty() && message.contains("object"))))
                    })
            })
    }

    pub(super) fn remember(
        &self,
        github: &GitHub<'_>,
        value: &Value,
        acquired: Instant,
    ) -> Result<()> {
        if let Some(errors) = value
            .get("errors")
            .filter(|e| e.as_array().is_none_or(|e| !e.is_empty()))
        {
            bail!("failed publication discovery: {errors}");
        }
        let repository = value["data"]
            .get("repository")
            .filter(|v| v.is_object())
            .context("missing publication repository")?;
        // Optional discovery may be unavailable on a compatible service; never infer absence.
        let Some(name) = repository.get("nameWithOwner").and_then(Value::as_str) else {
            github.disable_discovery();
            return Ok(());
        };
        if !name.eq_ignore_ascii_case(&github.config.repo.name) {
            bail!("publication repository identity mismatch");
        }
        let mut tips = Tips::new();
        for (index, reference) in self.references.iter().enumerate() {
            let node = repository
                .get(format!("publicationRef{index}"))
                .context("incomplete publication ref observation")?;
            let tip = if node.is_null() {
                None
            } else {
                let full = format!(
                    "{}{}",
                    node["prefix"].as_str().context("missing ref prefix")?,
                    node["name"].as_str().context("missing ref name")?
                );
                if full != format!("refs/heads/{reference}") {
                    bail!("publication ref identity mismatch");
                }
                Some(commit_oid(&node["target"])?.to_owned())
            };
            tips.insert(reference.clone(), tip);
        }
        let control = repository
            .get("publicationControl")
            .filter(|v| !v.is_null())
            .map(commit_oid)
            .transpose()?;
        let mut absent = Vec::new();
        for (index, hash) in self.objects.iter().enumerate() {
            let node = repository
                .get(format!("publicationObject{index}"))
                .context("incomplete commit presence observation")?;
            if node.is_null() {
                if control.is_some() {
                    absent.push(hash.clone());
                }
            } else if commit_oid(node)? != hash {
                bail!("commit presence identity mismatch");
            }
        }
        github.commands.explain(&format!(
            "Publication evidence: {} exact refs, {} commit checks, {} confirmed absent commits.",
            tips.len(),
            self.objects.len(),
            absent.len()
        ));
        let mut discovery = guard(&github.session.discovery);
        for (reference, tip) in tips {
            discovery.tips.insert(reference, (acquired, tip));
        }
        discovery
            .pending
            .retain(|hash| !self.objects.contains(hash));
        drop(discovery);
        let mut cache = guard(&github.session.commit_prs);
        for hash in absent {
            cache.insert(hash, (acquired, Vec::new().into()));
        }
        drop(cache);
        Ok(())
    }
}

fn commit_oid(value: &Value) -> Result<&str> {
    let oid = value["oid"]
        .as_str()
        .context("missing observed commit object")?;
    if value["__typename"] != "Commit" || !crate::git::policy::valid_oid(oid) {
        bail!("invalid observed commit object");
    }
    Ok(oid)
}

#[cfg(test)]
#[path = "../tests/unit/publication_discovery.rs"]
mod unit;
