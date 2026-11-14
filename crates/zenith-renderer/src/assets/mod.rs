//! Assets: ids, files, the registry and the parsing into CPU structures.
//!
//! Nothing here uses wgpu or the rest of the crate, so the module can become a
//! crate of its own.

pub mod container;
pub mod id;
pub mod import;
pub mod json;
pub mod ops;
pub mod registry;
