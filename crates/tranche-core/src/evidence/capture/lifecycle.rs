//! Starting a run, and the order the work happens in.
//!
//! Two checks bracket the acquisition. A live revision check runs first, because it
//! decides whether recorded code evidence may be reused at all. The final identity
//! checks run last, against a budget held back for exactly that, because sources may
//! not be certified against a revision that moved while the run was working.

use serde_json::{Value, json};

use super::super::read::transport;
use super::super::selection::Selection;
use super::super::{COMPONENTS, RESERVED_BUDGET};
use super::endpoints::groups_for;
use super::outcome::{MAX_PAGES, Stopped, Verify};
use crate::report::Root;

/// One capture run over one manifest.
pub struct Capture<'a> {
    pub(crate) root: &'a Root,
    pub(crate) selection: Selection,
    pub(crate) capture_id: String,
    pub(crate) manifest: Value,
    pub(crate) budget: transport::Budget,
    pub(crate) max_bytes: u64,
    pub(crate) max_pages: u64,
    pub(crate) stopped: Stopped,
    pub(crate) reused: u64,
    pub(crate) fetched: u64,
    pub log: Vec<String>,
}
impl<'a> Capture<'a> {
    pub fn new(
        root: &'a Root,
        selection: Selection,
        capture_id: String,
        manifest: Value,
        request_limit: u64,
        max_bytes: u64,
    ) -> Result<Self, String> {
        Ok(Self {
            root,
            selection,
            capture_id,
            manifest,
            budget: transport::Budget::new(request_limit, RESERVED_BUDGET as u64)
                .map_err(|error| error.message)?,
            max_bytes,
            max_pages: MAX_PAGES,
            stopped: Stopped::Finished,
            reused: 0,
            fetched: 0,
            log: Vec::new(),
        })
    }

    /// Acquire everything missing, reusing what the live revision allows.
    pub fn run(&mut self) -> Stopped {
        if self.manifest["capture"]["stop_reason"].as_str() == Some("revision_drift") {
            self.stopped = Stopped::RevisionDrift;
            return self.stopped.clone();
        }
        match self.acquire() {
            Ok(()) => Stopped::Finished,
            Err(stopped) => stopped,
        }
    }
    fn acquire(&mut self) -> Result<(), Stopped> {
        // The live revision check decides whether any recorded code evidence may be
        // reused at all, so it runs before the work that reserved capacity protects.
        let members = self.selection.members.clone();
        for member in &members {
            if !self.budget.can(Some(self.budget.reserved)) {
                return Err(Stopped::Budget);
            }
            let metadata = match self.verify_member(member, None) {
                Ok(metadata) => metadata,
                Err(Verify::Moved) => return Err(Stopped::RevisionDrift),
                Err(Verify::Failed(reason)) => {
                    self.log.push(format!("#{}: {reason}", member.number));
                    self.stopped = Stopped::Transport;
                    return Err(Stopped::Transport);
                }
            };
            // A thread-only update refreshes the mutable observations and leaves the
            // code observation alone: the diffs still describe the same two commits.
            if let Some(live) = metadata.get("updated_at").and_then(Value::as_str) {
                let recorded = self.manifest["code_observation"]["thread_updated_at"]
                    .get(member.number.to_string())
                    .and_then(Value::as_str);
                if recorded != Some(live) {
                    self.refresh_mutable(member, live);
                }
            }
        }
        self.checkpoint();

        for member in &members {
            for component in COMPONENTS {
                for group in groups_for(member, component) {
                    if self.group_status(member.number, component, &group) == "complete" {
                        self.reused += 1;
                        continue;
                    }
                    self.acquire_group(member, component, &group)?;
                    self.checkpoint();
                }
            }
        }

        // The held-back capacity is spent only now: sources may not be certified
        // against a revision that moved while the run was working.
        for member in &members {
            match self.verify_member(member, Some(0)) {
                Ok(_) | Err(Verify::Failed(_)) => {}
                Err(Verify::Moved) => {
                    self.manifest["capture"]["stop_reason"] = json!("revision_drift");
                    self.checkpoint();
                    self.stopped = Stopped::RevisionDrift;
                    return Err(Stopped::RevisionDrift);
                }
            }
        }
        self.derive_citations()?;
        self.checkpoint();
        Ok(())
    }

    /// Derive and record the citations, from the stored bytes.
    fn derive_citations(&mut self) -> Result<(), Stopped> {
        match super::super::citations::extract_citations(self.root, &self.manifest) {
            Ok(citations) => {
                self.manifest["citations"] = serde_json::json!(citations);
                Ok(())
            }
            Err(reason) => {
                self.log
                    .push(format!("citation derivation failed: {reason}"));
                Err(Stopped::Transport)
            }
        }
    }
}
