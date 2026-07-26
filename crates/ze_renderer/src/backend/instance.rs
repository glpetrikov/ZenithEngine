use bytemuck::{Pod, Zeroable};

/// Per-sprite data uploaded as instanced vertex attributes (locations 2..=9),
/// alongside the shared unit-quad mesh (locations 0..=1, per-vertex). One
/// instanced `draw_indexed` per texture group replaces the old one-draw-call-
/// per-sprite loop; everything that used to live in the per-object UBO
/// (`transform`) or the per-sprite material UBO (`tint`/`params`/`emissive`/
/// `uv_rect`) now travels here instead.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpriteInstanceRaw {
	/// Column-major, matching `glam::Mat4::to_cols_array_2d`.
	pub model: [[f32; 4]; 4],
	pub uv_rect: [f32; 4],
	pub tint: [f32; 4],
	pub params: [f32; 4],
	pub emissive: [f32; 4],
}

impl SpriteInstanceRaw {
	pub const fn get_layout() -> wgpu::VertexBufferLayout<'static> {
		const ATTRIBUTES: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
			2 => Float32x4,
			3 => Float32x4,
			4 => Float32x4,
			5 => Float32x4,
			6 => Float32x4,
			7 => Float32x4,
			8 => Float32x4,
			9 => Float32x4,
		];

		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<Self>() as u64,
			step_mode: wgpu::VertexStepMode::Instance,
			attributes: &ATTRIBUTES,
		}
	}
}
