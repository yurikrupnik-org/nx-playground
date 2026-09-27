//! Taskfile graph domain behind `taskgraph` (CLI) and `taskgraph_api`.
//!
//! - [`taskfile`] — parse a go-task v3 Taskfile (+ local includes) into a
//!   [`contract_taskgraph::Graph`].
//! - [`graph`] — dependency queries and the drill-down tree.
//! - [`observe`] — turn `task --verbose` stderr into execution facts.
//! - [`projection`] — fold facts into runs, per-task history and views.
//! - [`estimate`] — self-time statistics recombined along the graph.
//! - [`nats`] — publish to / replay from the `TASKGRAPH` stream.
//!
//! No HTTP, no database: the CLI and the API compose these pieces.

pub mod error;
pub mod estimate;
pub mod graph;
pub mod nats;
pub mod observe;
pub mod projection;
pub mod taskfile;

pub use error::{TaskgraphError, TaskgraphResult};
pub use estimate::{Estimate, Sample, TaskStats};
pub use graph::{GraphIndex, TreeNode};
pub use nats::{EventPublisher, NatsPublisher, NoopPublisher};
pub use observe::Tracker;
pub use projection::{Limits, Projection};
