mod error;
mod share_manager;
mod state;

pub type DpssID = usize;

pub use error::DpssError;

pub use state::DpssEpochState;

pub use share_manager::HandoffManager;
