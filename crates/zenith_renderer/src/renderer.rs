//! The scene and everything that draws it. It knows nothing about windows: it
//! is given a device and a texture view to draw into, so a window
//! ([`Viewport`](crate::viewport::Viewport)) and a test without one use it the
//! same way.

use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;
use zenith_error::ZResult;
use zenith_filesystem::{Project, ZPath};

use crate::{
	camera::{Camera, CameraController, CameraInput, CameraUniform, Projection},
	instance::{Instance, InstanceRaw},
	light::Light,
	model::{Material, Model, ModelVertex},
	render_pass::{DepthDesc, PipelineDesc, RenderPass, create_render_pipeline},
	texture::Texture,
};

/// The model of the scene, and where it lies in the project.
const MODEL_PATH: &str = "Assets://Models/Cube/cube.obj";

/// The copies of the model stand in a square of this many per side.
const INSTANCES_PER_ROW: u32 = 10;
/// How far from each other, along both axes.
const INSTANCE_SPACING: f32 = 3.0;

pub struct Renderer {
	device: wgpu::Device,
	queue: wgpu::Queue,

	depth_texture: Texture,

	camera: Camera,
	projection: Projection,
	camera_controller: CameraController,
	camera_uniform: CameraUniform,
	camera_buffer: wgpu::Buffer,
	camera_bind_group: wgpu::BindGroup,

	light: Light,

	model: Model,
	instances: Vec<Instance>,
	instance_buffer: wgpu::Buffer,

	scene_pass: RenderPass,
	light_pipeline: wgpu::RenderPipeline,
}

impl Renderer {
	/// ## Creates the renderer
	///
	/// `color_format` is the format of the view that [`Renderer::render`] will
	/// be given.
	///
	/// ## Errors
	/// The project's error if the model or a texture cannot be read, and an
	/// error if they are not valid.
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		color_format: wgpu::TextureFormat,
		width: u32,
		height: u32,
		project: &Project,
		label: &str,
	) -> ZResult<Self> {
		let material_layout = Material::bind_group_layout(device);

		let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			}],
			label: Some("camera_bind_group_layout"),
		});
		let light_bind_group_layout = Light::bind_group_layout(device);

		let camera = Camera::new(
			Vec3::new(0.0, 5.0, 10.0),
			-90.0_f32.to_radians(),
			-20.0_f32.to_radians(),
		);
		let projection = Projection::new(width, height, 45.0_f32.to_radians(), 0.1, 1000.0);
		let camera_controller = CameraController::new(6.0, 0.003);

		let mut camera_uniform = CameraUniform::new();
		camera_uniform.update_view_proj(&camera, &projection);

		let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some(&format!("{label} Camera Buffer")),
			contents: bytemuck::bytes_of(&camera_uniform),
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
		});
		let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			layout: &camera_bind_group_layout,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: camera_buffer.as_entire_binding(),
			}],
			label: Some("camera_bind_group"),
		});

		let light = Light::new(device, &light_bind_group_layout, Vec3::new(8.0, 6.0, 8.0), Vec3::ONE);

		let model = Model::load(device, queue, project, &ZPath::new(MODEL_PATH)?, &material_layout)?;

		let half = INSTANCES_PER_ROW as f32 * 0.5;
		let instances = (0..INSTANCES_PER_ROW)
			.flat_map(|z| {
				(0..INSTANCES_PER_ROW).map(move |x| {
					let position = Vec3::new(
						(x as f32 + 0.5 - half) * INSTANCE_SPACING,
						0.0,
						(z as f32 + 0.5 - half) * INSTANCE_SPACING,
					);
					// Rotating around a zero-length axis is not possible, and
					// the copy standing on the origin has none to go by.
					let rotation = if position == Vec3::ZERO {
						Quat::IDENTITY
					} else {
						Quat::from_axis_angle(position.normalize(), 45.0_f32.to_radians())
					};
					Instance { position, rotation }
				})
			})
			.collect::<Vec<_>>();
		let instance_data = instances.iter().map(Instance::to_raw).collect::<Vec<_>>();
		let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some(&format!("{label} Instance Buffer")),
			contents: bytemuck::cast_slice(&instance_data),
			usage: wgpu::BufferUsages::VERTEX,
		});

		let depth_texture = Texture::create_depth_texture(device, width, height, &format!("{label} Depth Texture"));

		let scene_pass = {
			let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
				label: Some("Scene Pipeline Layout"),
				bind_group_layouts: &[
					Some(&material_layout),
					Some(&camera_bind_group_layout),
					Some(&light_bind_group_layout),
				],
				immediate_size: 0,
			});
			let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/shader.wgsl"));
			let pipeline = create_render_pipeline(
				device,
				&PipelineDesc {
					label: "Scene Pipeline",
					layout: &layout,
					shader: &shader,
					vertex_layouts: &[ModelVertex::desc(), InstanceRaw::desc()],
					color_format,
					depth: Some(DepthDesc::SOLID),
					cull_mode: Some(wgpu::Face::Back),
					vs_entry: "vs_main",
					fs_entry: "fs_main",
				},
			);
			RenderPass::new("Scene", pipeline)
		};

		let light_pipeline = {
			let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
				label: Some("Light Pipeline Layout"),
				bind_group_layouts: &[Some(&camera_bind_group_layout), Some(&light_bind_group_layout)],
				immediate_size: 0,
			});
			let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/light.wgsl"));
			create_render_pipeline(
				device,
				&PipelineDesc {
					label: "Light Pipeline",
					layout: &layout,
					shader: &shader,
					vertex_layouts: &[ModelVertex::desc()],
					color_format,
					depth: Some(DepthDesc::SOLID),
					cull_mode: Some(wgpu::Face::Back),
					vs_entry: "vs_main",
					fs_entry: "fs_main",
				},
			)
		};

		Ok(Self {
			device: device.clone(),
			queue: queue.clone(),

			depth_texture,

			camera,
			projection,
			camera_controller,
			camera_uniform,
			camera_buffer,
			camera_bind_group,

			light,

			model,
			instances,
			instance_buffer,

			scene_pass,
			light_pipeline,
		})
	}

	/// The size of what is drawn into changed.
	pub fn resize(&mut self, width: u32, height: u32) {
		self.projection.resize(width, height);
		self.depth_texture = Texture::create_depth_texture(&self.device, width, height, "Depth Texture");
	}

	pub const fn camera(&self) -> &Camera {
		&self.camera
	}

	pub const fn camera_mut(&mut self) -> &mut Camera {
		&mut self.camera
	}

	pub const fn camera_controller_mut(&mut self) -> &mut CameraController {
		&mut self.camera_controller
	}

	/// Moves the camera by what the player asked, turns the light, and sends
	/// both to the GPU.
	pub fn update(&mut self, dt: f32, input: &CameraInput) {
		self.camera_controller.update_camera(&mut self.camera, input, dt);
		self.camera_uniform.update_view_proj(&self.camera, &self.projection);
		self.queue
			.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&self.camera_uniform));

		self.light.orbit(&self.queue, dt);
	}

	/// Records the drawing of the scene into `output`.
	pub fn render(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView, label: &str) {
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some(label),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: output,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: self.scene_pass.load_op,
					store: wgpu::StoreOp::Store,
				},
			})],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &self.depth_texture.view,
				depth_ops: Some(wgpu::Operations {
					load: wgpu::LoadOp::Clear(1.0),
					store: wgpu::StoreOp::Store,
				}),
				stencil_ops: None,
			}),
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		});

		// The lamp.
		pass.set_pipeline(&self.light_pipeline);
		pass.set_bind_group(0, &self.camera_bind_group, &[]);
		pass.set_bind_group(1, &self.light.bind_group, &[]);
		self.model.draw_geometry(&mut pass, 0..1);

		// The copies of the model.
		pass.set_pipeline(&self.scene_pass.pipeline);
		pass.set_bind_group(1, &self.camera_bind_group, &[]);
		pass.set_bind_group(2, &self.light.bind_group, &[]);
		pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
		self.model.draw_instanced(&mut pass, 0..self.instances.len() as u32);
	}
}
