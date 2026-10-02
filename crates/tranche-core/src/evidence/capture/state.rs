//! The run's bookkeeping: identity checks, refresh, the budget floor, the checkpoint
//! and the small accessors the rest reads through.
//!
//! The checkpoint is what makes a capture resumable, so it is written after every group
//! rather than only at the end.

use serde_json::{Value, json};

use super::super::now;
use super::super::read::transport;
use super::super::selection::Member;
use super::checks::is_exhausted;
use super::endpoints::{JSON_MEDIA, source_url};
use super::lifecycle::Capture;
use super::outcome::{Stopped, Verify};
use super::records;

impl<'a> Capture<'a> {
    pub(crate) fn verify_member(
        &mut self,
        member: &Member,
        reserve: Option<u64>,
    ) -> Result<Value, Verify> {
        self.budget.identity_checks += 1;
        let at = source_url(member, "metadata", "metadata");
        let response = transport::get(
            &at,
            Some(JSON_MEDIA),
            Some(&mut self.budget),
            reserve,
            Some(&member.base_repo_name),
        )
        .map_err(|error| {
            if is_exhausted(&error.message) {
                Verify::Failed("the request budget is spent".to_owned())
            } else {
                Verify::Failed(error.message)
            }
        })?;
        let data = response
            .json()
            .map_err(|error| Verify::Failed(error.message))?;
        if data.get("number").and_then(Value::as_u64) != Some(member.number) {
            return Err(Verify::Failed(format!(
                "live metadata for #{} reports a different pull request",
                member.number
            )));
        }
        let live_base = data
            .get("base")
            .and_then(|base| base.get("sha"))
            .and_then(Value::as_str);
        let live_head = data
            .get("head")
            .and_then(|head| head.get("sha"))
            .and_then(Value::as_str);
        let (Some(live_base), Some(live_head)) = (live_base, live_head) else {
            return Err(Verify::Failed(format!(
                "#{} no longer exposes its revisions",
                member.number
            )));
        };
        if live_base != member.base_sha || live_head != member.head_sha {
            return Err(Verify::Moved);
        }
        Ok(data)
    }
    pub(crate) fn refresh_mutable(&mut self, member: &Member, thread_updated: &str) {
        for component in super::super::MUTABLE_COMPONENTS {
            let entry = records::component_entry(&mut self.manifest, member.number, component);
            if let Some(groups) = entry["groups"].as_array_mut() {
                for state in groups {
                    state["status"] = json!("missing");
                    state["reason"] = json!(format!(
                        "thread updated upstream ({thread_updated}); refresh pending"
                    ));
                }
            }
            super::super::coverage::roll_up(entry);
        }
        self.manifest["code_observation"]["thread_updated_at"][member.number.to_string()] =
            json!(thread_updated);
    }
    pub(crate) fn reserve(&mut self) -> Result<(), Stopped> {
        if !self.budget.can(Some(self.budget.reserved)) {
            self.manifest["capture"]["stop_reason"] = json!("request_budget");
            return Err(Stopped::Budget);
        }
        Ok(())
    }
    pub(crate) fn checkpoint(&mut self) {
        let used = self.budget.used;
        let failures = self.budget.failures;
        let reserved = self.budget.reserved;
        let checks = self.budget.identity_checks;
        let capture = &mut self.manifest["capture"];
        capture["requests_used"] = json!(used);
        capture["failures"] = json!(failures);
        capture["reserved"] = json!(reserved);
        capture["identity_checks"] = json!(checks);
        match self.stopped.reason() {
            Some(reason) => capture["stop_reason"] = json!(reason),
            // A resumed capture that got past the point where it ran out of budget
            // is no longer stopped by it; a stop reason describes the last run.
            None if capture["stop_reason"].as_str() == Some("request_budget") => {
                capture["stop_reason"] = Value::Null;
            }
            None => {}
        }
        capture["observed_at"] = json!(now());
        let _ =
            super::super::manifest::write_manifest(self.root, &self.capture_id, &mut self.manifest);
    }
    pub(crate) fn block(&mut self, number: u64, component: &str, group: &str, reason: &str) {
        records::block(&mut self.manifest, number, component, group, reason);
    }

    pub(crate) fn group_status(&self, number: u64, component: &str, group: &str) -> String {
        self.group(number, component, group)["status"]
            .as_str()
            .unwrap_or("missing")
            .to_owned()
    }

    pub(crate) fn group_pages(&self, number: u64, component: &str, group: &str) -> u64 {
        self.group(number, component, group)["pages"]
            .as_u64()
            .unwrap_or(0)
    }

    pub(crate) fn group_next_url(
        &self,
        number: u64,
        component: &str,
        group: &str,
    ) -> Option<String> {
        self.group(number, component, group)["next_url"]
            .as_str()
            .map(str::to_owned)
    }

    pub(crate) fn group(&self, number: u64, component: &str, group: &str) -> &Value {
        self.manifest["components"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| {
                entry["number"].as_u64() == Some(number)
                    && entry["component"].as_str() == Some(component)
            })
            .and_then(|entry| {
                entry["groups"]
                    .as_array()?
                    .iter()
                    .find(|state| state["group"].as_str() == Some(group))
            })
            .unwrap_or(&Value::Null)
    }

    pub(crate) fn over_storage(&self) -> bool {
        self.manifest["capture"]["bytes_stored"]
            .as_u64()
            .unwrap_or(0)
            > self.max_bytes
    }

    pub fn manifest(&self) -> &Value {
        &self.manifest
    }

    pub fn fetched(&self) -> u64 {
        self.fetched
    }

    pub fn reused(&self) -> u64 {
        self.reused
    }
}
