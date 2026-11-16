//! Materials on the GPU: bind group 1 of every pass that draws meshes.
//!
//! The layout is fixed here, once, and the shaders follow it:
//!
//! ```wgsl
//! struct Material {
//!     base_color: vec4f,   // linear RGBA
//!     roughness: f32,
//!     metallic: f32,
//!     alpha_cutoff: f32,   // 0 unless the material is `mask`
//!     flags: u32,          // MaterialUniform::MASKED, ...
//! }
//! @group(1) @binding(0) var<uniform> material: Material;
//! @group(1) @binding(1) var base_color_texture: texture_2d<f32>;
//! @group(1) @binding(2) var base_color_sampler: sampler;
//! @group(1) @binding(3) var normal_texture: texture_2d<f32>;
//! @group(1) @binding(4) var normal_sampler: sampler;
//! ```

use wgpu::util::DeviceExt;

use super::textures::GpuTexture;
use crate::assets::{
	AssetId,
	json::{BlendMode, MaterialData},
};

/// The group of the material in the passes that draw meshes.
pub const MATERIAL_GROUP: u32 = 1;

/// The uniform buffer of a material (binding 0).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MaterialUniform {
	pub base_color: [f32; 4],
	pub roughness: f32,
	pub metallic: f32,
	/// Pixels with less alpha are not drawn; 0 unless the material is `mask`.
	pub alpha_cutoff: f32,
	pub flags: u32,
}

impl MaterialUniform {
	/// The material is `mask`: the shader discards by `alpha_cutoff`.
	pub const MASKED: u32 = 1;

	pub fn of(data: &MaterialData) -> Self {
		let masked = data.blend == BlendMode::Mask;
		Self {
			base_color: data.base_color,
			roughness: data.roughness,
			metallic: data.metallic,
			alpha_cutoff: if masked { data.alpha_cutoff } else { 0.0 },
			flags: if masked { Self::MASKED } else { 0 },
		}
	}
}

/// The entries of the layout of group 1.
pub fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 5] {
	let texture = |binding| wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::FRAGMENT,
		ty: wgpu::BindingType::Texture {
			sample_type: wgpu::TextureSampleType::Float { filterable: true },
			view_dimension: wgpu::TextureViewDimension::D2,
			multisampled: false,
		},
		count: None,
	};
	let sampler = |binding| wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::FRAGMENT,
		ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
		count: None,
	};
	[
		wgpu::BindGroupLayoutEntry {
			binding: 0,
			visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
			ty: wgpu::BindingType::Buffer {
				ty: wgpu::BufferBindingType::Uniform,
				has_dynamic_offset: false,
				min_binding_size: wgpu::BufferSize::new(size_of::<MaterialUniform>() as u64),
			},
			count: None,
		},
		texture(1),
		sampler(2),
		texture(3),
		sampler(4),
	]
}

/// A material on the GPU.
#[derive(Debug)]
pub struct GpuMaterial {
	pub bind_group: wgpu::BindGroup,
	pub uniform: wgpu::Buffer,
	/// For the key of the pipeline: blending and culling.
	pub blend: BlendMode,
	pub double_sided: bool,
	pub shader: String,
	/// The textures it uses: they are kept while it is used.
	pub textures: Vec<AssetId>,
}

impl GpuMaterial {
	/// A material of `data` with these textures (stand-ins for the ones it
	/// has not, or that could not be loaded).
	pub fn create(
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		label: &str,
		data: &MaterialData,
		base_color: &GpuTexture,
		normal: &GpuTexture,
	) -> Self {
		let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some(label),
			contents: bytemuck::bytes_of(&MaterialUniform::of(data)),
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
		});
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some(label),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: uniform.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&base_color.view),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::Sampler(&base_color.sampler),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(&normal.view),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::Sampler(&normal.sampler),
				},
			],
		});
		Self {
			bind_group,
			uniform,
			blend: data.blend,
			double_sided: data.double_sided,
			shader: data.shader.clone(),
			textures: data.base_color_texture.into_iter().chain(data.normal_texture).collect(),
		}
	}

	/// What it takes of the memory of the GPU (its textures are counted on
	/// their own).
	pub fn bytes(&self) -> u64 {
		self.uniform.size()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_cutoff_is_for_a_masked_material_only() {
		let opaque = MaterialUniform::of(&MaterialData::default());
		assert_eq!(opaque.flags, 0);
		assert!(opaque.alpha_cutoff.abs() < f32::EPSILON, "nothing is discarded");
		let masked = MaterialUniform::of(&MaterialData {
			blend: BlendMode::Mask,
			alpha_cutoff: 0.3,
			..MaterialData::default()
		});
		assert_eq!(masked.flags, MaterialUniform::MASKED);
		assert!((masked.alpha_cutoff - 0.3).abs() < f32::EPSILON);
		assert_eq!(
			size_of::<MaterialUniform>(),
			32,
			"a WGSL struct of vec4f and four scalars"
		);
	}
}
