//! Loading and validating the deployment contract.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The deployment contract read from `tranche.json` at the report root.
#[derive(Debug, Clone)]
pub struct Contract {
    /// The reviewed repository, `owner/name` — the identity every binding and
    /// gate carries.
    repository: String,
    /// The model alias recorded in bindings and provenance.
    model: String,
    /// The frozen question contract: `{"judge": {...}, "pair": {...}}`.
    policy: Value,
    /// Display metadata: title, links, category labels. Free to change.
    display: Value,
    /// Where the file was read from, for error messages.
    path: PathBuf,
}

impl Contract {
    /// The contract file every deployment carries at its report root.
    pub const FILE_NAME: &'static str = "tranche.json";

    /// Read `tranche.json` under `root`.
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(Self::FILE_NAME);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {}", path.display(), error))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        Self::from_value(value, path)
    }

    /// Build the contract from an already-parsed value. Tests use this.
    pub fn from_value(value: Value, path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let field = |name: &str| -> Result<&Value, String> {
            value
                .get(name)
                .ok_or_else(|| format!("{}: missing `{name}`", path.display()))
        };
        let text = |name: &str| -> Result<String, String> {
            field(name)?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{}: `{name}` must be a string", path.display()))
        };
        let repository = text("repository")?;
        if !repository.contains('/') || repository.split('/').any(str::is_empty) {
            return Err(format!(
                "{}: `repository` must be `owner/name`, got `{repository}`",
                path.display()
            ));
        }
        let model = text("model")?;
        let policy = field("policy")?.clone();
        for half in ["judge", "pair"] {
            if policy.get(half).and_then(Value::as_object).is_none() {
                return Err(format!(
                    "{}: `policy.{half}` must be an object of questions",
                    path.display()
                ));
            }
        }
        for (question, spec) in policy["judge"].as_object().expect("checked above").iter() {
            if spec.get("type").and_then(Value::as_str).is_none() {
                return Err(format!(
                    "{}: `policy.judge.{question}.type` must be a string",
                    path.display()
                ));
            }
        }
        let display = value.get("display").cloned().unwrap_or_else(|| {
            serde_json::json!({
                "title": format!("TRIAGE with Tranche — {repository}"),
                "repo_url": format!("https://github.com/{repository}"),
            })
        });
        Ok(Self {
            repository,
            model,
            policy,
            display,
            path,
        })
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// The judge questions exactly as stored — the binding digests this value.
    pub fn judge_questions(&self) -> &Value {
        self.policy.get("judge").expect("validated at load")
    }

    /// The pair questions exactly as stored.
    pub fn pair_questions(&self) -> &Value {
        self.policy.get("pair").expect("validated at load")
    }

    /// The `sameness`-style choice labels, in the order the policy lists them.
    ///
    /// Verdict normalization keeps only the choices the policy names; the order
    /// is the policy's own insertion order, which the report preserves.
    pub fn pair_choices(&self) -> Vec<String> {
        self.pair_questions()["sameness"]["criteria"]
            .as_object()
            .map(|criteria| criteria.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn display(&self) -> &Value {
        &self.display
    }

    /// Where the contract was read from, for error messages.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A display string with a fallback, so a minimal contract still renders.
    pub fn display_str(&self, key: &str, fallback: &str) -> String {
        self.display
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    }

    /// The name reviewer prompts address: the display `subject`, or the
    /// repository itself when the deployment named none.
    pub fn batch_subject(&self) -> String {
        self.display_str("subject", &self.repository)
    }
}
