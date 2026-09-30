//! Docker interaction, isolated in one module (Section 8) so shell commands are never scattered
//! across the codebase.

pub mod client;
pub mod container_runner;
pub mod health;
pub mod image_manager;
pub mod runner;

pub use health::{doctor, require_ready, DockerHealth};
pub use image_manager::{ensure_image, image_for_major};
pub use runner::DockerPgRunner;
