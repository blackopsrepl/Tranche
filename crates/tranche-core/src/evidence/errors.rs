//! Evidence failures, kept as one type so a command can map them to the exact exit
//! codes scripts already branch on.
//!
//! The three kinds are distinct on purpose. A refusal is a request that cannot be
//! satisfied as asked; a selection failure means the bound report no longer describes
//! what is being asked of it; an incomplete capture is usable and merely unfinished,
//! which is why it is not an error to the caller who only wanted what was captured.

use std::fmt;

/// The evidence request cannot be satisfied as asked.
#[derive(Debug)]
pub struct EvidenceError(pub String);

impl fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EvidenceError {}

/// The current native selection refuses this request.
#[derive(Debug)]
pub struct SelectionError(pub String);

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SelectionError {}

/// The capture is usable but not complete; the message says how to resume.
#[derive(Debug)]
pub struct IncompleteCapture(pub String);

impl fmt::Display for IncompleteCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for IncompleteCapture {}

/// Every evidence failure, in the one type a command dispatches on.
#[derive(Debug)]
pub enum Error {
    Evidence(EvidenceError),
    Selection(SelectionError),
    Incomplete(IncompleteCapture),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evidence(error) => write!(f, "{error}"),
            Self::Selection(error) => write!(f, "{error}"),
            Self::Incomplete(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<EvidenceError> for Error {
    fn from(value: EvidenceError) -> Self {
        Self::Evidence(value)
    }
}

impl From<SelectionError> for Error {
    fn from(value: SelectionError) -> Self {
        Self::Selection(value)
    }
}

impl From<IncompleteCapture> for Error {
    fn from(value: IncompleteCapture) -> Self {
        Self::Incomplete(value)
    }
}

/// A refusal with a message, in the narrowest type that reports it.
pub fn refuse(message: impl Into<String>) -> EvidenceError {
    EvidenceError(message.into())
}

/// A refusal from the bound report rather than from the capture.
pub fn selection_refusal(message: impl Into<String>) -> SelectionError {
    SelectionError(message.into())
}
