pub mod error;
pub mod ids;

pub use error::{ClusterError, Result};
pub use ids::{NodeId, JobId, UserId};

pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");
