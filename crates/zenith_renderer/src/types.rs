pub enum PowerMode {
	Economy,
	Performance,
}

pub struct SurfaceFormats {
	pub storage: wgpu::TextureFormat,
	pub view: wgpu::TextureFormat,
}
