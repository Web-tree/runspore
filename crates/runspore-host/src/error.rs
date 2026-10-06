use runspore_types::reducer::Failure;
use runspore_types::store::StoreFailure;

/// An error of the engine's public API. Store and kernel failures keep their codes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// The store refused or could not perform an operation.
    #[error("store: {0}")]
    Store(#[from] StoreFailure),
    /// The kernel rejected a transition.
    #[error("kernel: {0}")]
    Kernel(#[from] Failure),
    /// The request was rejected by the engine before it reached the store.
    #[error("{code}: {message}")]
    Invalid { code: String, message: String },
}

impl HostError {
    pub(crate) fn invalid(code: &str, message: impl Into<String>) -> Self {
        Self::Invalid {
            code: code.to_string(),
            message: message.into(),
        }
    }

    /// The machine-readable failure code: the store's, the kernel's, or the engine's.
    pub fn code(&self) -> &str {
        match self {
            Self::Store(f) => &f.code,
            Self::Kernel(f) => &f.code,
            Self::Invalid { code, .. } => code,
        }
    }
}
