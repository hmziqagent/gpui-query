use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Clones share one flag: cancelling any clone cancels all of them.
#[derive(Debug, Clone)]
pub struct QuerySignal {
    cancelled: Arc<AtomicBool>,
}

impl PartialEq for QuerySignal {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

impl Eq for QuerySignal {}

impl QuerySignal {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for QuerySignal {
    fn default() -> Self {
        Self::new()
    }
}
