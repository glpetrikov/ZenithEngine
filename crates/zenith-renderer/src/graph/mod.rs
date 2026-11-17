//! The render graph: its description, compilation and execution.
//!
//! A graph is data ([`GraphDesc`], JSON): nodes of passes from a
//! [`PassRegistry`] and the links between their slots. [`compile`] checks it
//! and orders it into a [`CompiledGraph`], which runs for every view that
//! uses it, with its textures from a [`TexturePool`].

pub mod compile;
pub mod desc;
pub mod exec;
pub mod pass;
pub mod pool;
pub mod registry;

pub use compile::{CompiledGraph, Diagnostic, Severity, compile};
pub use desc::{GraphDesc, LinkDesc, NodeDesc, NodeId, SlotRef};
pub use exec::{ExecContext, Frame, InitContext, PrepareContext, ViewInfo, ViewKey};
pub use pass::{
	Access, FormatRule, InputDesc, OutputDesc, OutputSource, ParamDesc, ParamMap, ParamType, ParamValue, PassAction,
	PassCategory, PassDesc, PassRegistry, RenderPass, SizeRule, SlotKind, TextureDesc,
};
pub use pool::{TextureKey, TexturePool};
pub use registry::{GraphEntry, GraphId, GraphRegistry};
