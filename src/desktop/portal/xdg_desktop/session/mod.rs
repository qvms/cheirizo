//! Session management namespace for runtime portal sessions.
//!
//! Defines session lifecycle/state types and re-exports manager APIs used by
//! the portal D-Bus and backend orchestration layers.

mod manager;
mod state;

pub use manager::{SessionManager, SessionManagerConfig};
pub use state::{PersistMode, RestoreData, Session, SessionState};
