//! What is made ready for the GPU between extract and render: the assets of
//! the frame ([`assets`]), the matrices and buffers of every view
//! ([`views`]) and what each view draws, sorted and batched
//! ([`draw_lists`]).

pub mod assets;
pub mod draw_lists;
pub mod lights;
pub mod sprite_lists;
pub mod views;

pub use assets::ProjectRes;
pub use draw_lists::{DrawLists, InstanceData, MeshBatch, MeshList, ViewDrawLists};
pub use lights::LightsUniform;
pub use sprite_lists::{SpriteBatch, SpriteInstance, SpriteList, SpriteLists};
pub use views::{CameraUniform, PreparedView, PreparedViews};
