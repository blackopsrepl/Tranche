//! The answer schema every policy shares.
//!
//! A deployment's `tranche.json` decides what is asked; this decides how an
//! answer is read once given. The two halves have different lifecycles — the
//! policy is a frozen contract whose edit re-bills the corpus, this is engine
//! semantics that only change with a `BINDING_VERSION` bump — which is why the
//! schema stays code while the questions became data.

/// A metric's ceiling, which decides whether a model answer is usable.
///
/// Absent, non-finite or out-of-range values are unknown, never zero.
pub fn metric_ceiling(name: &str, field: &str) -> Option<f64> {
    match field {
        "noul" => Some(1.0),
        "score" if name == "risk" => Some(4.0),
        "score" => Some(3.0),
        _ => None,
    }
}

/// The field a question's answer is read from.
pub fn answer_field(question_type: &str) -> Option<&'static str> {
    match question_type {
        "choice" => Some("choice"),
        "score" => Some("score"),
        "noul" => Some("noul"),
        _ => None,
    }
}
