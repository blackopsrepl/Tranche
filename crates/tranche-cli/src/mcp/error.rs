//! MCP failures, reported through tool results rather than the protocol.

/// The error type the MCP surface reports through tool results.
#[derive(Debug, Clone)]
pub struct ReportError(pub String);

impl std::fmt::Display for ReportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ReportError {}

/// The arguments one `query` call carries, as one type rather than ten.
pub struct QueryArguments<'a> {
    pub text: &'a str,
    pub category: Option<&'a str>,
    pub risk_band: Option<&'a str>,
    pub security: Option<bool>,
    pub finished_form: Option<f64>,
    pub batch: Option<&'a str>,
    pub queue: &'a str,
    pub offset: u64,
    pub limit: u64,
}
