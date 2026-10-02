//! Reading one group: a REST list, or the closing-issues query.
//!
//! A group that cannot be read is blocked and named rather than failing the run. Half a
//! capture with an accurate gap is more useful than no capture with a silent one.

use super::super::read::transport;
use super::super::selection::Member;
use super::checks::{is_exhausted, verify_ci_scope, with_params};
use super::endpoints::{JSON_MEDIA, accept_for, repo_prefix, source_url};
use super::lifecycle::Capture;
use super::outcome::{MAX_DIFF_BYTES, MAX_DIFF_LINES, Stopped};
use super::pages::{self, Counts};
use super::records;

impl<'a> Capture<'a> {
    pub(crate) fn acquire_group(
        &mut self,
        member: &Member,
        component: &str,
        group: &str,
    ) -> Result<(), Stopped> {
        if component == "closing_issues" {
            return self.acquire_closing(member);
        }
        self.acquire_rest(member, component, group)
    }
    pub(crate) fn acquire_rest(
        &mut self,
        member: &Member,
        component: &str,
        group: &str,
    ) -> Result<(), Stopped> {
        let mut counts = Counts::default();
        let accept = accept_for(component);
        let prefix = repo_prefix(member, component, group);
        let mut page = self.group_pages(member.number, component, group) + 1;
        let mut at = self
            .group_next_url(member.number, component, group)
            .unwrap_or_else(|| source_url(member, component, group));

        loop {
            self.reserve()?;
            // Metadata and the diff take no pagination parameters; sending them
            // would be a different request than the one the URL names.
            let reserve = self.budget.reserved;
            let url = with_params(&at, component, group, page);
            let response = match transport::get(
                &url,
                Some(accept),
                Some(&mut self.budget),
                Some(reserve),
                Some(&prefix),
            ) {
                Ok(response) => response,
                Err(error) if error.too_large => {
                    self.block(
                        member.number,
                        component,
                        group,
                        &format!("response exceeds the transport bound: {}", error.message),
                    );
                    return Ok(());
                }
                Err(error) if is_exhausted(&error.message) => {
                    self.stopped = Stopped::Budget;
                    return Err(Stopped::Budget);
                }
                Err(error) => {
                    self.block(
                        member.number,
                        component,
                        group,
                        &format!("acquisition failed: {}", error.message),
                    );
                    return Ok(());
                }
            };

            // A CI response names the repository it came from, and a
            // repository-prefix check on the URL is not proof that it is the
            // resource that was asked for.
            if component == "checks"
                && let Err(reason) = verify_ci_scope(member, group, &response)
            {
                self.block(member.number, component, group, &reason);
                return Ok(());
            }

            let accepted = if component == "diff" {
                pages::diff(&response, MAX_DIFF_BYTES, MAX_DIFF_LINES)
            } else {
                pages::list(member, component, group, &response, page, &mut counts)
            };
            let accepted = match accepted {
                Ok(accepted) => accepted,
                Err(reason) => {
                    self.block(member.number, component, group, &reason);
                    return Ok(());
                }
            };

            // The media type is owned because the record borrows it.
            let media_type = response.media_type();
            let page_text = page.to_string();
            let cursor = if page > 1 {
                Some(page_text.as_str())
            } else {
                None
            };
            let recorded = records::record(
                self.root,
                &mut self.manifest,
                records::Recorded {
                    number: member.number,
                    component,
                    group,
                    body: &response.body,
                    url: &response.url,
                    accept,
                    media_type: &media_type,
                    cursor,
                    next_cursor: accepted.next.as_deref(),
                },
            );
            if let Err(reason) = recorded {
                self.block(member.number, component, group, &reason);
                return Ok(());
            }
            self.fetched += 1;
            records::set_counts(&mut self.manifest, member.number, component, group, &counts);

            if self.over_storage() {
                records::finish(
                    &mut self.manifest,
                    member.number,
                    component,
                    group,
                    false,
                    Some("incomplete: stopped at the storage bound"),
                );
                self.stopped = Stopped::Budget;
                return Err(Stopped::Budget);
            }

            if accepted.complete {
                let reason = pages::shortfall(component, group, &counts);
                records::finish(
                    &mut self.manifest,
                    member.number,
                    component,
                    group,
                    reason.is_none(),
                    reason.as_deref(),
                );
                return Ok(());
            }
            let Some(next) = accepted.next else {
                records::finish(
                    &mut self.manifest,
                    member.number,
                    component,
                    group,
                    false,
                    Some("incomplete: no continuation"),
                );
                return Ok(());
            };
            if page >= self.max_pages {
                records::finish(
                    &mut self.manifest,
                    member.number,
                    component,
                    group,
                    false,
                    Some(&format!(
                        "incomplete: stopped at the {}-page bound",
                        self.max_pages
                    )),
                );
                return Ok(());
            }
            at = next;
            page += 1;
        }
    }
    pub(crate) fn acquire_closing(&mut self, member: &Member) -> Result<(), Stopped> {
        let mut cursor: Option<String> = None;
        let mut page = self.group_pages(member.number, "closing_issues", "closing_issues") + 1;
        loop {
            self.reserve()?;
            let variables = super::endpoints::closing_variables(member, cursor.as_deref());
            let reserve = self.budget.reserved;
            let response = match transport::graphql(
                super::endpoints::closing_query(),
                &variables,
                Some(&mut self.budget),
                Some(reserve),
            ) {
                Ok(response) => response,
                Err(error) if is_exhausted(&error.message) => {
                    self.stopped = Stopped::Budget;
                    return Err(Stopped::Budget);
                }
                Err(error) => {
                    self.block(
                        member.number,
                        "closing_issues",
                        "closing_issues",
                        &format!("acquisition failed: {}", error.message),
                    );
                    return Ok(());
                }
            };
            let payload = response.json().map_err(|_| Stopped::Transport)?;
            let (next, observed) = match super::endpoints::parse_closing(&payload, member) {
                Ok(parsed) => parsed,
                Err(reason) => {
                    self.block(member.number, "closing_issues", "closing_issues", &reason);
                    return Ok(());
                }
            };
            let media_type = response.media_type();
            let recorded = records::record(
                self.root,
                &mut self.manifest,
                records::Recorded {
                    number: member.number,
                    component: "closing_issues",
                    group: "closing_issues",
                    body: &response.body,
                    url: &response.url,
                    accept: JSON_MEDIA,
                    media_type: &media_type,
                    cursor: cursor.as_deref(),
                    next_cursor: next.as_deref(),
                },
            );
            if let Err(reason) = recorded {
                self.block(member.number, "closing_issues", "closing_issues", &reason);
                return Ok(());
            }
            self.fetched += 1;
            records::set_counts(
                &mut self.manifest,
                member.number,
                "closing_issues",
                "closing_issues",
                &Counts {
                    observed,
                    reported: None,
                },
            );

            match next {
                None => {
                    records::finish(
                        &mut self.manifest,
                        member.number,
                        "closing_issues",
                        "closing_issues",
                        true,
                        None,
                    );
                    return Ok(());
                }
                Some(_next) if page >= self.max_pages => {
                    records::finish(
                        &mut self.manifest,
                        member.number,
                        "closing_issues",
                        "closing_issues",
                        false,
                        Some(&format!(
                            "incomplete: stopped at the {}-page bound",
                            self.max_pages
                        )),
                    );
                    return Ok(());
                }
                Some(next) => {
                    cursor = Some(next);
                    page += 1;
                }
            }
        }
    }
}
