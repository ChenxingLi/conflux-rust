pub mod poll_filter;
pub mod poll_manager;

pub use poll_filter::{
    limit_logs, PollFilter, SyncPollFilter, MAX_BLOCK_HISTORY_SIZE,
};
pub use poll_manager::PollManager;

pub const MAX_FEE_HISTORY_CACHE_BLOCK_COUNT: u64 = 1024;
