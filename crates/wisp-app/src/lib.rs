//! Host-independent application services for Wisp clients.
//!
//! Hosts supply an existing store and snapshots of their live state. Services
//! return shared `wisp-dto` data without owning windows, an IPC transport, or
//! another database. Migrate services here one use case at a time.

pub mod mandates;
pub mod projects;
