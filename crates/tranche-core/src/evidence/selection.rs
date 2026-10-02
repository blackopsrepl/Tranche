//! The selection a capture generation is bound to, and the manifest validation
//! that proves a stored checkpoint describes itself.
//!
//! A manifest is untrusted input. It is a file under `out/` that a writer could
//! have edited, so reading one re-derives every identity it claims — the
//! generation from its own selection, each source's digest from its own record —
//! rather than believing the stored value.

use serde_json::{Map, Value, json};

use super::{Error, FORMAT, PROFILE};
use crate::util::digest;

/// Full 40-character commit SHA.
pub fn is_sha1(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A batch id such as `B001`.
pub fn is_batch_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (4..=7).contains(&bytes.len()) && bytes[0] == b'B' && bytes[1..].iter().all(u8::is_ascii_digit)
}

/// The code identity of one member: the exact commits and the repositories they
/// sit in.
#[derive(Debug, Clone)]
pub struct Member {
    pub number: u64,
    pub source_digest: String,
    pub evidence_digest: String,
    pub base_sha: String,
    pub head_sha: String,
    pub base_repo_id: u64,
    pub base_repo_name: String,
    pub head_repo_id: u64,
    pub head_repo_name: String,
    pub head_fork: bool,
    pub updated_at: Option<String>,
}

impl Member {
    /// What a revision move means: commits and the repositories they sit in.
    ///
    /// `updated_at` is deliberately excluded. It is observed upstream data, and a
    /// PR update is thread activity rather than a code revision — treating it as
    /// a revision marker would force re-downloading unchanged code.
    pub fn revision_key(&self) -> Value {
        json!({
            "base_sha": self.base_sha,
            "head_sha": self.head_sha,
            "base_repo_id": self.base_repo_id,
            "base_repo_name": self.base_repo_name,
            "head_repo_id": self.head_repo_id,
            "head_repo_name": self.head_repo_name,
        })
    }

    fn as_json(&self) -> Value {
        json!({
            "number": self.number,
            "source_digest": self.source_digest,
            "evidence_digest": self.evidence_digest,
            "base_sha": self.base_sha,
            "head_sha": self.head_sha,
            "base_repo_id": self.base_repo_id,
            "base_repo_name": self.base_repo_name,
            "head_repo_id": self.head_repo_id,
            "head_repo_name": self.head_repo_name,
            "head_fork": self.head_fork,
            "updated_at": self.updated_at,
        })
    }

    fn from_json(value: &Value) -> Result<Self, Error> {
        let read_str = |key: &str| -> Result<String, Error> {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| super::refuse(format!("member is missing {key}")).into())
        };
        let read_int = |key: &str| -> Result<u64, Error> {
            value
                .get(key)
                .and_then(Value::as_u64)
                .ok_or_else(|| super::refuse(format!("member is missing {key}")).into())
        };
        Ok(Self {
            number: read_int("number")?,
            source_digest: read_str("source_digest")?,
            evidence_digest: read_str("evidence_digest")?,
            base_sha: read_str("base_sha")?,
            head_sha: read_str("head_sha")?,
            base_repo_id: read_int("base_repo_id")?,
            base_repo_name: read_str("base_repo_name")?,
            head_repo_id: read_int("head_repo_id")?,
            head_repo_name: read_str("head_repo_name")?,
            head_fork: value
                .get("head_fork")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            updated_at: value
                .get("updated_at")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// The immutable native selection one capture generation is bound to.
#[derive(Debug, Clone)]
pub struct Selection {
    pub repository: Value,
    pub report: Value,
    pub batch_id: String,
    pub batch_ordinal: u64,
    pub prompt: String,
    pub members: Vec<Member>,
}

impl Selection {
    pub fn numbers(&self) -> Vec<u64> {
        self.members.iter().map(|member| member.number).collect()
    }

    /// The frozen projection written into every manifest and export.
    ///
    /// The native batch is copied whole — ordinal, ordered membership and the
    /// exact reviewer prompt — because the prompt is provenance a reader can
    /// check, not a cache key. Freezing only these fields keeps a later
    /// presentation change from invalidating recorded evidence.
    pub fn as_json(&self) -> Value {
        json!({
            "repository": self.repository,
            "report": self.report,
            "batch": {
                "id": self.batch_id,
                "ordinal": self.batch_ordinal,
                "members": self.numbers(),
                "count": self.members.len(),
                "review_prompt": self.prompt,
            },
            "members": self.members.iter().map(Member::as_json).collect::<Vec<_>>(),
        })
    }

    /// The order-sensitive membership digest.
    pub fn membership_digest(&self) -> String {
        digest(&json!(self.numbers()))
    }

    /// The code identity of the whole selection, stable across thread churn.
    pub fn revision(&self) -> Value {
        let mut map = Map::new();
        for member in &self.members {
            map.insert(member.number.to_string(), member.revision_key());
        }
        Value::Object(map)
    }

    /// One member by number.
    pub fn member(&self, number: u64) -> Result<&Member, Error> {
        self.members
            .iter()
            .find(|member| member.number == number)
            .ok_or_else(|| {
                super::refuse(format!("#{number} is not a member of this selection")).into()
            })
    }
}

/// The capture generation: what all its sources belong to and cannot escape.
///
/// It deliberately excludes `updated_at`: a thread update must not mint a new
/// generation or force re-downloading unchanged code, where a moved base or head
/// must.
pub fn generation_id(selection: &Selection, capture_id: &str) -> String {
    digest(&json!({
        "format": FORMAT,
        "profile": PROFILE,
        "capture_id": capture_id,
        "repository": selection.repository,
        "revision": selection.revision(),
        "membership": selection.membership_digest(),
        "batch": selection.batch_id,
        "report": selection.report,
    }))
}

/// The digest one source record's identity is derived from.
pub fn source_id(generation: &str, source: &Value) -> String {
    let fields: Map<String, Value> = source
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter(|(key, _)| key.as_str() != "id")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    digest(&json!({"generation": generation, "source": Value::Object(fields)}))
}

/// Rebuild a selection from its frozen JSON, refusing a malformed projection.
pub fn selection_from_json(stored: &Value) -> Result<Selection, Error> {
    let object = stored
        .as_object()
        .ok_or_else(|| super::refuse("capture manifest selection is malformed"))?;
    let keys: std::collections::HashSet<&str> = object.keys().map(String::as_str).collect();
    if keys
        != ["repository", "report", "batch", "members"]
            .into_iter()
            .collect()
    {
        return Err(super::refuse("capture manifest selection is malformed").into());
    }
    let batch = object.get("batch").expect("checked above");
    let batch_object = batch
        .as_object()
        .ok_or_else(|| super::refuse("capture manifest batch provenance is malformed"))?;
    let batch_keys: std::collections::HashSet<&str> =
        batch_object.keys().map(String::as_str).collect();
    if batch_keys
        != ["id", "ordinal", "members", "count", "review_prompt"]
            .into_iter()
            .collect()
    {
        return Err(super::refuse("capture manifest batch provenance is malformed").into());
    }
    let ordinal = batch_object
        .get("ordinal")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let id = batch_object.get("id").and_then(Value::as_str).unwrap_or("");
    let prompt = batch_object
        .get("review_prompt")
        .and_then(Value::as_str)
        .unwrap_or("");
    let batch_members: Vec<u64> = batch_object
        .get("members")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let count = batch_object
        .get("count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if prompt.trim().is_empty()
        || batch_members.is_empty()
        || count != batch_members.len() as u64
        || id != format!("B{ordinal:03}")
    {
        return Err(super::refuse("capture manifest batch provenance is malformed").into());
    }

    let members_json = object
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| super::refuse("capture manifest membership does not match its batch"))?;
    let mut members = Vec::new();
    for entry in members_json {
        let member = Member::from_json(entry)?;
        if !is_sha1(&member.base_sha) || !is_sha1(&member.head_sha) {
            return Err(super::refuse("capture manifest member revision is malformed").into());
        }
        members.push(member);
    }
    if members
        .iter()
        .map(|member| member.number)
        .collect::<Vec<_>>()
        != batch_members
    {
        return Err(super::refuse("capture manifest membership does not match its batch").into());
    }

    Ok(Selection {
        repository: object.get("repository").cloned().unwrap_or(Value::Null),
        report: object.get("report").cloned().unwrap_or(Value::Null),
        batch_id: id.to_owned(),
        batch_ordinal: ordinal,
        prompt: prompt.to_owned(),
        members,
    })
}
