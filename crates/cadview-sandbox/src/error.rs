#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("component loading failed: {0}")]
    ComponentLoad(#[source] anyhow::Error),

    #[error("instantiation failed: {0}")]
    Instantiation(#[source] anyhow::Error),

    #[error("execution failed: {0}")]
    Execution(#[source] anyhow::Error),

    #[error("execution timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("program error: {0}")]
    ProgramError(String),
}
