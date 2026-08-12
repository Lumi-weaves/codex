//! Local, browser-facing projections for the Lumi Codex Web management UI.
//!
//! This crate owns narrow Web DTOs. It never exposes generic app-server RPC.

mod agent_operations;

pub use agent_operations::AgentOperationNode;
pub use agent_operations::AgentOperationRole;
pub use agent_operations::AgentOperationStatus;
pub use agent_operations::AgentOperationsError;
pub use agent_operations::AgentOperationsService;
pub use agent_operations::AgentOperationsSnapshot;
