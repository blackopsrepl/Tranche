//! The invalid-input constructor the loader and the checker both use.

use super::error::PrError;

pub(crate) fn invalid(error: impl std::fmt::Display) -> PrError {
    PrError(format!("{error}"))
}
