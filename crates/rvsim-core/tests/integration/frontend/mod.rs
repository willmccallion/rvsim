//! Fetch, branch prediction and the front end's redirects.

pub mod btb_frontend;
pub mod btb_training;
pub mod fetch_buffer;
pub mod fetch_inflight_limit;
pub mod fetch_line_straddle;
pub mod fetch_page_crossing;
pub mod fetch_walk;
pub mod frontend_prediction;
pub mod jump_history;
pub mod line_crossing;
pub mod misaligned_target;
pub mod page_crossing;
pub mod squash_history;
