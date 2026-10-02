use serde::{Deserialize, Serialize};

/// Inert config: focus/reconnect refetching is not implemented yet, so this
/// is stored but never acted on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefetchTrigger {
    #[default]
    Always,
    IfStale,
    Never,
}

impl RefetchTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::Always => "Always",
            Self::IfStale => "If stale",
            Self::Never => "Never",
        }
    }
}
