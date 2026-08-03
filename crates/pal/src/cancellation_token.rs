use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cloneable cancellation flag shared across task execution boundaries.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a token in the non-cancelled state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation for all holders of this token.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;

    #[test]
    fn cloned_tokens_observe_cancellation() {
        let token = CancellationToken::new();
        let clone = token.clone();

        assert!(!clone.is_cancelled());

        token.cancel();

        assert!(clone.is_cancelled());
    }
}
