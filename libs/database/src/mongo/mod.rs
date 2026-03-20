//! MongoDB connector module
//!
//! Provides connection utilities for MongoDB.

pub mod connector;

pub use connector::connect;
pub use mongodb::{Client, Collection, Database};
