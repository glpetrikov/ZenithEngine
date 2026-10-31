use glam::{Vec2, Vec3};

pub enum PowerMode {
	Economy,
	Performance,
}

pub struct SurfaceFormats {
	pub storage: wgpu::TextureFormat,
	pub view: wgpu::TextureFormat,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
	position: Vec3,
	tex_coords: Vec2,

	_pad: f32,
}

impl Vertex {
	pub const fn new(position: Vec3, tex_coords: Vec2) -> Self {
		Self {
			tex_coords,
			position,
			_pad: 0.0,
		}
	}

	pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &[
				wgpu::VertexAttribute {
					offset: std::mem::offset_of!(Self, position) as wgpu::BufferAddress,
					shader_location: 0,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: std::mem::offset_of!(Self, tex_coords) as wgpu::BufferAddress,
					shader_location: 1,
					format: wgpu::VertexFormat::Float32x2,
				},
			],
		}
	}
}

pub const VERTICES: &[Vertex] = &[
	Vertex::new(
		Vec3::new(-0.086_824_1, 0.492_403_86, 0.0),
		Vec2::new(0.413_175_9, 0.007_596_14),
	),
	Vertex::new(
		Vec3::new(-0.495_134_06, 0.069_586_47, 0.0),
		Vec2::new(0.004_865_944_4, 0.430_413_54),
	),
	Vertex::new(
		Vec3::new(-0.219_185_49, -0.449_397_06, 0.0),
		Vec2::new(0.280_814_53, 0.949_397),
	),
	Vertex::new(
		Vec3::new(0.359_669_98, -0.347_329_1, 0.0),
		Vec2::new(0.859_67, 0.847_329_14),
	),
	Vertex::new(
		Vec3::new(0.441_473_72, 0.234_735_9, 0.0),
		Vec2::new(0.941_473_7, 0.265_264_1),
	),
];

pub const INDICES: &[u16] = &[0, 1, 4, 1, 2, 4, 2, 3, 4];
