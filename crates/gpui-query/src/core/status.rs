use serde::{Deserialize, Serialize};

/// `Idle` → `LoadingEmpty` → `Success`/`Failure`; refetch: `Success` → `LoadingWithData` → terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum QueryStatus {
    #[default]
    Idle,
    LoadingEmpty,
    LoadingWithData,
    Success,
    Failure,
    Cancelled,
}

impl QueryStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::LoadingEmpty => "Loading empty",
            Self::LoadingWithData => "Loading with data",
            Self::Success => "Success",
            Self::Failure => "Failure",
            Self::Cancelled => "Cancelled",
        }
    }

    pub fn is_loading(self) -> bool {
        matches!(self, Self::LoadingEmpty | Self::LoadingWithData)
    }

    /// TanStack Query's `isPending`: no data yet (`Idle` or first load).
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Idle | Self::LoadingEmpty)
    }
}
