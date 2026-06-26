//! L0 session errors. Typed so callers branch on a machine-readable code.

#[derive(thiserror::Error, Debug)]
pub enum SessionError {
    #[error("cli_session.cli_not_found: {0}")]
    CliNotFound(String),
    #[error("cli_session.not_authenticated: {0}")]
    NotAuthenticated(String),
    #[error("cli_session.spawn_failed: {0}")]
    SpawnFailed(String),
    #[error("cli_session.timeout: {0}")]
    Timeout(String),
    #[error("cli_session.transport: {0}")]
    Transport(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn error_messages_carry_a_code_prefix() {
        let e = SessionError::CliNotFound("agy".into());
        assert!(format!("{e}").starts_with("cli_session.cli_not_found"));
    }
}
