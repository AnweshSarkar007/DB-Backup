//! The backup pipeline: engine state machine plus its helpers (naming, hashing, manifest, locking).

pub mod engine;
pub mod hash;
pub mod lock;
pub mod manifest;
pub mod naming;

pub use engine::{run_backup, EngineOptions, Progress};
