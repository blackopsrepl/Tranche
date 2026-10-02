//! The transport could not produce a usable response.

/// An oversized response is distinguished because the caller's remedy differs: the
/// bound is raised, not the request retried.
#[derive(Debug, Clone)]
pub struct ReadError {
    pub message: String,
    pub too_large: bool,
}

impl ReadError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            too_large: false,
        }
    }

    pub fn too_large(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            too_large: true,
        }
    }
}

/// Read one URL, following validated redirects and charging every attempt.
pub fn read(
    url: &str,
    accept: Option<&str>,
    budget: Option<&mut super::transport::Budget>,
    repo_prefix: Option<&str>,
) -> Result<super::transport::Response, ReadError> {
    super::transport::get(url, accept, budget, None, repo_prefix)
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ReadError {}
