//! A bounded, GET-only read transport for evidence acquisition.
//!
//! Reading a PR list and throwing the status, headers and bytes away is enough for
//! discovery. Evidence needs all three, so this is the narrow extension: the same
//! credential ownership (`gh` holds the token, this process never reads it and
//! never puts it in an argv) while returning the exact response.
//!
//! Everything here is read-only. REST requests are explicit GET; a GraphQL
//! document is accepted only if it is a query, and the document travels on stdin
//! so it never appears in `ps` output. Redirects and continuation links are
//! validated against the allowed origin and the repository scope before they are
//! followed, and every attempt is charged to a budget.

pub mod error;
pub mod gh_call;
pub mod transport;
pub mod url;

pub use error::{ReadError, read};
pub use transport::{Budget, MAX_RESPONSE_BYTES, Response};
