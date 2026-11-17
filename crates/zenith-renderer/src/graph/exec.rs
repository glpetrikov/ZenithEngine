//! Running a `CompiledGraph` for a view, and the contexts a pass is given.
//!
//! For every view: the textures of the graph are taken from the pool, every
//! pass prepares (and may skip), then every pass that runs records its
//! commands, in a debug group named after its node. The graph decides how a
//! texture is loaded: the pass that creates it clears it, the others load
//! it; the target of the view is cleared only by the first camera on it.

use std::{any::Any, collections::HashMap};

use bevy_ecs::world::World;

use super::{
	compile::{Binding, CompiledGraph, NodeState, Resource, ResourceKind},
	pass::PassAction,
	pool::{TextureKey, TexturePool},
};
use crate::cache::GpuCache;

/// A view, for the state a pass keeps per view: the same number every frame
/// for the same camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewKey(pub u64);

/// What a graph is run for.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo<'a> {
	pub key: ViewKey,
	/// What the camera draws into: a window, a texture.
	pub target: &'a wgpu::TextureView,
	pub target_format: wgpu::TextureFormat,
	pub target_size: (u32, u32),
	/// The part of the target, in 0..1 (x, y, width, height); `None`: all of
	/// it.
	pub viewport: Option<[f32; 4]>,
	/// `Some`: the first camera on its target this frame, which clears it to
	/// this color. `None`: it draws over the camera before.
	pub clear: Option<wgpu::Color>,
}

impl ViewInfo<'_> {
	/// The viewport in pixels: x, y, width, height; at least 1 a side.
	#[allow(clippy::cast_sign_loss)] // the shares are clamped to 0..1 first
	pub fn viewport_pixels(&self) -> [u32; 4] {
		let (width, height) = self.target_size;
		let [x, y, w, h] = self.viewport.unwrap_or([0.0, 0.0, 1.0, 1.0]);
		let px = |share: f32, of: u32| ((share.clamp(0.0, 1.0) * of as f32).round() as u32).min(of);
		let (x, y) = (px(x, width), px(y, height));
		[
			x,
			y,
			px(w, width).clamp(1, width.saturating_sub(x).max(1)),
			px(h, height).clamp(1, height.saturating_sub(y).max(1)),
		]
	}

	/// The size the textures of the graph are made for: that of the viewport.
	pub fn size(&self) -> (u32, u32) {
		let [_, _, width, height] = self.viewport_pixels();
		(width, height)
	}
}

/// What a frame lends the graphs.
pub struct Frame<'a> {
	pub device: &'a wgpu::Device,
	pub queue: &'a wgpu::Queue,
	pub cache: &'a mut GpuCache,
	pub world: &'a World,
	pub pool: &'a mut TexturePool,
	/// One encoder for the frame: every view records into it.
	pub encoder: &'a mut wgpu::CommandEncoder,
	/// The number of the frame.
	pub number: u64,
}

/// What a pass is given in [`RenderPass::init`](super::pass::RenderPass::init).
pub struct InitContext<'a> {
	pub device: &'a wgpu::Device,
	pub queue: &'a wgpu::Queue,
	pub cache: &'a mut GpuCache,
}

type Values = [Option<Box<dyn Any + Send + Sync>>];

/// The slots of a node.
#[derive(Clone, Copy)]
struct Slots<'a> {
	inputs: &'a HashMap<String, Binding>,
	outputs: &'a HashMap<String, Binding>,
}

impl Slots<'_> {
	/// An output of that name, else an input.
	fn get(&self, slot: &str) -> Option<Binding> {
		self.outputs.get(slot).or_else(|| self.inputs.get(slot)).copied()
	}

	/// An output of the node: what it may put a note on or publish data in.
	fn own(&self, slot: &str) -> Option<Binding> {
		self.outputs.get(slot).copied()
	}
}

/// The format and size of a texture of the graph in this view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextureInfo {
	format: wgpu::TextureFormat,
	size: (u32, u32),
}

/// What a pass is given in
/// [`RenderPass::prepare`](super::pass::RenderPass::prepare).
pub struct PrepareContext<'a> {
	pub device: &'a wgpu::Device,
	pub queue: &'a wgpu::Queue,
	pub cache: &'a mut GpuCache,
	/// For the input nodes, and a pass of a game that needs what no node
	/// gives. The other passes read their slots.
	pub world: &'a World,
	pub view: &'a ViewInfo<'a>,
	pub frame: u64,
	slots: Slots<'a>,
	textures: &'a [Option<TextureInfo>],
	data: &'a mut Values,
	notes: &'a mut Values,
	bind_groups: &'a mut BindGroupCache,
	view_states: &'a mut HashMap<ViewKey, Box<dyn Any + Send + Sync>>,
}

impl PrepareContext<'_> {
	/// The format of the texture of `slot`.
	pub fn texture_format(&self, slot: &str) -> Option<wgpu::TextureFormat> {
		self.texture_info(slot).map(|info| info.format)
	}

	/// The size of the texture of `slot`, in pixels.
	pub fn texture_size(&self, slot: &str) -> Option<(u32, u32)> {
		self.texture_info(slot).map(|info| info.size)
	}

	fn texture_info(&self, slot: &str) -> Option<TextureInfo> {
		*self.textures.get(self.slots.get(slot)?.resource.0)?
	}

	/// The data of `slot` (an input linked to a data output, or an output of
	/// this pass), if it was published as a `T`.
	pub fn data<T: Any>(&self, slot: &str) -> Option<&T> {
		value(self.data, self.slots.get(slot)?)
	}

	/// Publishes the data of the output `slot`. `false`: the pass has no such
	/// output, and nothing is published.
	pub fn set_data<T: Any + Send + Sync>(&mut self, slot: &str, value: T) -> bool {
		put(self.data, self.slots.own(slot), value)
	}

	/// Puts a note on the texture of the output `slot`, for the passes that
	/// read it ([`ExecContext::note`]). `false`: no such output.
	pub fn set_note<T: Any + Send + Sync>(&mut self, slot: &str, note: T) -> bool {
		put(self.notes, self.slots.own(slot), note)
	}

	/// A note on the texture of `slot`.
	pub fn note<T: Any>(&self, slot: &str) -> Option<&T> {
		value(self.notes, self.slots.get(slot)?)
	}

	/// A bind group of `layout` with `entries`: the same one as long as what
	/// is in it is the same (a texture made again makes a new one).
	pub fn bind_group(
		&mut self,
		layout: &wgpu::BindGroupLayout,
		entries: &[wgpu::BindGroupEntry<'_>],
	) -> wgpu::BindGroup {
		self.bind_groups.get(self.device, layout, entries, self.frame)
	}

	/// The state of this pass for this view, made with `T::default()` the
	/// first time (and again if it was of another type).
	///
	/// # Panics
	/// Never: the state is made a `T` just before it is taken.
	#[allow(clippy::expect_used)] // it was just made a `T`
	pub fn view_state<T: Default + Send + Sync + 'static>(&mut self) -> &mut T {
		let state = self
			.view_states
			.entry(self.view.key)
			.or_insert_with(|| Box::new(T::default()));
		if !state.is::<T>() {
			*state = Box::new(T::default());
		}
		state.downcast_mut::<T>().expect("the state is a T")
	}
}

fn value<T: Any>(values: &Values, binding: Binding) -> Option<&T> {
	values.get(binding.resource.0)?.as_ref()?.downcast_ref::<T>()
}

fn put<T: Any + Send + Sync>(values: &mut Values, binding: Option<Binding>, value: T) -> bool {
	let Some(slot) = binding.and_then(|binding| values.get_mut(binding.resource.0)) else {
		return false;
	};
	*slot = Some(Box::new(value));
	true
}

/// What a pass is given in [`RenderPass::exec`](super::pass::RenderPass::exec).
pub struct ExecContext<'a> {
	pub encoder: &'a mut wgpu::CommandEncoder,
	pub world: &'a World,
	pub cache: &'a GpuCache,
	pub view: &'a ViewInfo<'a>,
	slots: Slots<'a>,
	resources: &'a [Resource],
	views: &'a [Option<wgpu::TextureView>],
	data: &'a Values,
	notes: &'a Values,
}

impl<'a> ExecContext<'a> {
	/// The texture of `slot`.
	pub fn texture(&self, slot: &str) -> Option<&'a wgpu::TextureView> {
		self.views.get(self.slots.get(slot)?.resource.0)?.as_ref()
	}

	/// The data of `slot`.
	pub fn data<T: Any>(&self, slot: &str) -> Option<&'a T> {
		value(self.data, self.slots.get(slot)?)
	}

	/// A note on the texture of `slot`.
	pub fn note<T: Any>(&self, slot: &str) -> Option<&'a T> {
		value(self.notes, self.slots.get(slot)?)
	}

	/// `slot` as a color attachment: cleared to `clear` if this pass is the
	/// first to draw into it (the target of the view: to the color of the
	/// camera, if it is the first on it), loaded otherwise.
	pub fn color_attachment(&self, slot: &str, clear: wgpu::Color) -> Option<wgpu::RenderPassColorAttachment<'a>> {
		color_attachment(self.slots, self.resources, self.views, self.view, slot, clear)
	}

	/// `slot` as a depth attachment, cleared to 1 by the first to draw into
	/// it.
	pub fn depth_attachment(&self, slot: &str) -> Option<wgpu::RenderPassDepthStencilAttachment<'a>> {
		depth_attachment(self.slots, self.views, slot)
	}

	/// A render pass into the color `slots` (each with the color it is
	/// cleared to, if it is) and the `depth` slot. Drawing into the target of
	/// the view sets the viewport of the camera. `None`: a slot that is not
	/// bound.
	pub fn begin_render_pass(
		&mut self,
		label: &str,
		colors: &[(&str, wgpu::Color)],
		depth: Option<&str>,
	) -> Option<wgpu::RenderPass<'_>> {
		let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'a>>> = colors
			.iter()
			.map(|&(slot, clear)| {
				color_attachment(self.slots, self.resources, self.views, self.view, slot, clear).map(Some)
			})
			.collect::<Option<_>>()?;
		let depth = match depth {
			Some(slot) => Some(depth_attachment(self.slots, self.views, slot)?),
			None => None,
		};
		let into_target = colors.iter().any(|&(slot, _)| {
			self.slots
				.get(slot)
				.is_some_and(|binding| self.resources[binding.resource.0].is_view_target())
		});
		let mut pass = self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some(label),
			color_attachments: &attachments,
			depth_stencil_attachment: depth,
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		});
		if into_target && self.view.viewport.is_some() {
			let [x, y, width, height] = self.view.viewport_pixels();
			pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
			pass.set_scissor_rect(x, y, width, height);
		}
		Some(pass)
	}
}

fn color_attachment<'a>(
	slots: Slots<'_>,
	resources: &[Resource],
	views: &'a [Option<wgpu::TextureView>],
	view: &ViewInfo<'_>,
	slot: &str,
	clear: wgpu::Color,
) -> Option<wgpu::RenderPassColorAttachment<'a>> {
	let binding = slots.get(slot)?;
	let load = if resources.get(binding.resource.0)?.is_view_target() {
		match (binding.clear, view.clear) {
			(true, Some(color)) => wgpu::LoadOp::Clear(color),
			_ => wgpu::LoadOp::Load,
		}
	} else if binding.clear {
		wgpu::LoadOp::Clear(clear)
	} else {
		wgpu::LoadOp::Load
	};
	Some(wgpu::RenderPassColorAttachment {
		view: views.get(binding.resource.0)?.as_ref()?,
		resolve_target: None,
		depth_slice: None,
		ops: wgpu::Operations {
			load,
			store: wgpu::StoreOp::Store,
		},
	})
}

fn depth_attachment<'a>(
	slots: Slots<'_>,
	views: &'a [Option<wgpu::TextureView>],
	slot: &str,
) -> Option<wgpu::RenderPassDepthStencilAttachment<'a>> {
	let binding = slots.get(slot)?;
	Some(wgpu::RenderPassDepthStencilAttachment {
		view: views.get(binding.resource.0)?.as_ref()?,
		depth_ops: Some(wgpu::Operations {
			load: if binding.clear {
				wgpu::LoadOp::Clear(1.0)
			} else {
				wgpu::LoadOp::Load
			},
			store: wgpu::StoreOp::Store,
		}),
		stencil_ops: None,
	})
}

/// The bind groups a pass made, by what is in them.
#[derive(Default)]
pub(crate) struct BindGroupCache {
	groups: HashMap<BindKey, (wgpu::BindGroup, u64)>,
}

#[derive(PartialEq, Eq, Hash)]
struct BindKey {
	layout: wgpu::BindGroupLayout,
	entries: Vec<(u32, BindResource)>,
}

#[derive(PartialEq, Eq, Hash)]
enum BindResource {
	Buffer(wgpu::Buffer, u64, Option<wgpu::BufferSize>),
	View(wgpu::TextureView),
	Sampler(wgpu::Sampler),
}

impl BindGroupCache {
	fn get(
		&mut self,
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		entries: &[wgpu::BindGroupEntry<'_>],
		frame: u64,
	) -> wgpu::BindGroup {
		let make = || {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("Zenith Graph Bind Group"),
				layout,
				entries,
			})
		};
		let key: Option<Vec<(u32, BindResource)>> = entries
			.iter()
			.map(|entry| {
				let resource = match &entry.resource {
					wgpu::BindingResource::Buffer(binding) => {
						BindResource::Buffer(binding.buffer.clone(), binding.offset, binding.size)
					}
					wgpu::BindingResource::TextureView(view) => BindResource::View((*view).clone()),
					wgpu::BindingResource::Sampler(sampler) => BindResource::Sampler((*sampler).clone()),
					// Arrays and the rest are made every time.
					_ => return None,
				};
				Some((entry.binding, resource))
			})
			.collect();
		let Some(entries) = key else {
			return make();
		};
		let key = BindKey {
			layout: layout.clone(),
			entries,
		};
		let (group, used) = self.groups.entry(key).or_insert_with(|| (make(), frame));
		*used = frame;
		group.clone()
	}

	/// Forgets what was not used for a few frames.
	fn prune(&mut self, frame: u64) {
		self.groups.retain(|_, (_, used)| frame.saturating_sub(*used) <= 2);
	}
}

impl CompiledGraph {
	/// Runs the graph for `view`: its passes record their commands into the
	/// encoder of `frame`. A pass that fails is logged once and left out; the
	/// frame goes on.
	pub fn run(&mut self, frame: &mut Frame<'_>, view: &ViewInfo<'_>) {
		for node in &mut self.nodes {
			if node.state == NodeState::New {
				let mut ctx = InitContext {
					device: frame.device,
					queue: frame.queue,
					cache: frame.cache,
				};
				node.state = match node.pass.init(&mut ctx) {
					Ok(()) => NodeState::Ready,
					Err(e) => {
						zenith_log::error!("The pass {} cannot work and is left out: {e}", node.label);
						NodeState::Broken
					}
				};
			}
		}

		// The textures.
		let size = view.size();
		let physical: Vec<(wgpu::TextureView, TextureInfo)> = self
			.physical
			.iter()
			.map(|&(format, rule)| {
				let format = format.resolve(view.target_format);
				let (width, height) = rule.pixels(size);
				let (_, texture) =
					frame
						.pool
						.acquire(frame.device, TextureKey::new(format, width, height), frame.number);
				(
					texture,
					TextureInfo {
						format,
						size: (width, height),
					},
				)
			})
			.collect();
		let target = TextureInfo {
			format: view.target_format,
			size,
		};
		let (views, textures): (Vec<Option<wgpu::TextureView>>, Vec<Option<TextureInfo>>) = self
			.resources
			.iter()
			.map(|resource| match resource.kind {
				ResourceKind::Texture {
					physical: Some(index), ..
				} => physical
					.get(index)
					.map_or((None, None), |(view, info)| (Some(view.clone()), Some(*info))),
				ResourceKind::Texture { physical: None, .. } => (Some(view.target.clone()), Some(target)),
				ResourceKind::Data => (None, None),
			})
			.unzip();
		let mut data: Vec<Option<Box<dyn Any + Send + Sync>>> = self.resources.iter().map(|_| None).collect();
		let mut notes: Vec<Option<Box<dyn Any + Send + Sync>>> = self.resources.iter().map(|_| None).collect();

		// Prepare.
		let mut actions = Vec::with_capacity(self.nodes.len());
		for node in &mut self.nodes {
			if node.state != NodeState::Ready {
				actions.push(PassAction::Skip);
				continue;
			}
			let mut ctx = PrepareContext {
				device: frame.device,
				queue: frame.queue,
				cache: frame.cache,
				world: frame.world,
				view,
				frame: frame.number,
				slots: Slots {
					inputs: &node.inputs,
					outputs: &node.outputs,
				},
				textures: &textures,
				data: &mut data,
				notes: &mut notes,
				bind_groups: &mut node.bind_groups,
				view_states: &mut node.view_states,
			};
			actions.push(node.pass.prepare(&mut ctx));
		}

		// Exec.
		for (node, action) in self.nodes.iter_mut().zip(actions) {
			if node.state != NodeState::Ready {
				continue;
			}
			let slots = Slots {
				inputs: &node.inputs,
				outputs: &node.outputs,
			};
			if action == PassAction::Skip {
				clear_created(frame.encoder, slots, &self.resources, &views, view);
				continue;
			}
			frame.encoder.push_debug_group(&node.label);
			let mut ctx = ExecContext {
				encoder: frame.encoder,
				world: frame.world,
				cache: frame.cache,
				view,
				slots,
				resources: &self.resources,
				views: &views,
				data: &data,
				notes: &notes,
			};
			if let Err(e) = node.pass.exec(&mut ctx)
				&& !node.reported
			{
				zenith_log::error!("The pass {} failed: {e}", node.label);
				node.reported = true;
			}
			frame.encoder.pop_debug_group();
			node.bind_groups.prune(frame.number);
		}
		frame.pool.release_all();
	}
}

/// What a skipped pass creates is cleared, so the passes after it read a
/// clean texture.
fn clear_created(
	encoder: &mut wgpu::CommandEncoder,
	slots: Slots<'_>,
	resources: &[Resource],
	views: &[Option<wgpu::TextureView>],
	view: &ViewInfo<'_>,
) {
	for (name, binding) in slots.outputs.iter().filter(|(_, binding)| binding.clear) {
		let is_depth = match resources.get(binding.resource.0).map(|r| &r.kind) {
			Some(ResourceKind::Texture { format, .. }) => format.resolve(view.target_format).is_depth_stencil_format(),
			_ => false,
		};
		if is_depth {
			let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(name),
				color_attachments: &[],
				depth_stencil_attachment: depth_attachment(slots, views, name),
				occlusion_query_set: None,
				timestamp_writes: None,
				multiview_mask: None,
			});
		} else if let Some(attachment) = color_attachment(slots, resources, views, view, name, wgpu::Color::TRANSPARENT)
			&& !matches!(attachment.ops.load, wgpu::LoadOp::Load)
		{
			let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(name),
				color_attachments: &[Some(attachment)],
				depth_stencil_attachment: None,
				occlusion_query_set: None,
				timestamp_writes: None,
				multiview_mask: None,
			});
		}
	}
}
