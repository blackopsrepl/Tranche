//! The deployment contract: which repository, which model, which questions.
//!
//! One JSON file at the report root, `tranche.json`, makes a checkout a
//! deployment. Everything the engine previously compiled in — the reviewed
//! repository, the model alias, the category taxonomy, the wording of every
//! question — is data here, so the same binary triages any repository's pull
//! requests.
//!
//! The file has two halves with different lifecycles:
//!
//! - `policy` is a frozen contract. Judgment and pair bindings digest it;
//!   editing a single character re-asks every question and re-bills the full
//!   backlog. It is loaded, never generated.
//! - `display` does not affect model-call bindings. Titles and labels render
//!   freely; a `subject` edit changes persisted reviewer prompts and requires
//!   offline batch regeneration before the report gate accepts those batches.

mod contract;
mod repository;
mod validation;

pub use repository::parse_repository;

pub use contract::Contract;
