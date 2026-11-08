use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use web_time::Instant;
#[cfg(target_arch = "wasm32")]
use winit::platform::web::EventLoopExtWebSys;
use winit::{
	application::ApplicationHandler,
	event::{DeviceEvent, DeviceId, WindowEvent},
	event_loop::{ActiveEventLoop, EventLoop},
	window::Window,
};
use zenith_error::{ZInternalResult, ZResult};
use zenith_input::{CursorMode, Gamepads, Input, ZKeyCode, ZMouseButton};
use zenith_renderer::{camera::CameraInput, types::PowerMode, viewport::Viewport};
use zenith_world::World;

/// The file of the world the runtime starts with.
const WORLD_FILE: &str = "Worlds://world.zenith";

/// The world of the project: its snapshot, `Worlds/world.zenith`, is read
/// through the project (a folder, or a build on the desktop and on the web)
/// and restored into a new world. A project without that file starts with an
/// empty world, and says so in the log.
///
/// # Errors
/// The project's error if the file cannot be read, and the world's if it is
/// not a world file.
fn load_world(project: &zenith_filesystem::Project) -> ZResult<World> {
	let mut world = World::new("world");
	let path = zenith_filesystem::ZPath::new(WORLD_FILE)?;
	if project.exists(&path)? {
		world.restore_bytes(&project.read_bytes(&path)?)?;
		zenith_log::info!("Loaded {path}");
	} else {
		zenith_log::warn!("{path} does not exist: starting with an empty world");
	}
	Ok(world)
}

/// The controls the author of the game chose: settings and actions.
const INPUT_SETTINGS: &str = "Settings://Input.toml";
/// The changes the player made to the controls, kept apart from the game.
const INPUT_OVERRIDES: &str = "Data://Input.toml";
/// How often the changes of the player are looked for and written.
const SAVE_OVERRIDES_EVERY: f32 = 2.0;

/// Reads the controls into the input of the world: the settings of the
/// project, then the changes of the player, if there are any. Returns the text
/// of the changes as they are now, which is what is on disk.
///
/// # Errors
/// The project's error if the settings cannot be read or are not valid. The
/// changes of the player never fail the start: a bad file is reported in the
/// log and ignored.
fn load_input(project: &zenith_filesystem::Project, world: &mut World) -> ZResult<String> {
	let settings = zenith_filesystem::ZPath::new(INPUT_SETTINGS)?;
	if project.exists(&settings)? {
		world.input_mut().load_settings(&project.read_string(&settings)?)?;
	}
	let overrides = zenith_filesystem::ZPath::new(INPUT_OVERRIDES)?;
	let loaded = project
		.exists(&overrides)
		.and_then(|exists| exists.then(|| project.read_string(&overrides)).transpose())
		.and_then(|text| text.map_or(Ok(()), |text| world.input_mut().load_overrides(&text)));
	if let Err(e) = loaded {
		zenith_log::warn!("The changes you made to the controls cannot be used: {e}");
	}
	Ok(world.input().overrides_toml())
}

/// What the player asks of the camera this frame: `W` `A` `S` `D` walk, `Space`
/// and `Shift` go up and down, the wheel moves along the view, and the mouse
/// turns the camera while the right button is held.
///
/// The right button also takes the cursor away (and gives it back), so the
/// mouse can turn the camera as far as it likes.
fn camera_input(input: &mut Input) -> CameraInput {
	let axis = |plus: ZKeyCode, minus: ZKeyCode| {
		f32::from(u8::from(input.is_key_pressed(plus))) - f32::from(u8::from(input.is_key_pressed(minus)))
	};
	let (forward, right, up) = (
		axis(ZKeyCode::W, ZKeyCode::S),
		axis(ZKeyCode::D, ZKeyCode::A),
		axis(ZKeyCode::Space, ZKeyCode::LeftShift),
	);

	if input.is_mouse_just_pressed(ZMouseButton::Right) {
		input.set_cursor_mode(CursorMode::Locked);
	}
	if input.is_mouse_just_released(ZMouseButton::Right) {
		input.set_cursor_mode(CursorMode::Normal);
	}
	let look = if input.is_mouse_pressed(ZMouseButton::Right) {
		input.mouse_delta()
	} else {
		glam::Vec2::ZERO
	};

	CameraInput {
		forward,
		right,
		up,
		look,
		scroll: input.scroll().y,
	}
}

pub struct State {
	viewport: Viewport,
	window: Arc<Window>,
	project: zenith_filesystem::Project,
	/// The world the game runs in; it holds the input.
	world: World,
	/// Real gamepads; `None` where the system has none to give.
	gamepads: Option<Gamepads>,
	last_frame: Instant,
	/// The changes to the controls that are on disk (or in the browser).
	saved_overrides: String,
	overrides_timer: f32,
}

impl State {
	/// Creates a new `State` instance with the given `Window`.
	///
	/// # Errors
	/// add in future.
	#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
	pub async fn new(window: Arc<Window>) -> ZResult<Self> {
		#[cfg(not(target_arch = "wasm32"))]
		let project = native::open_project()?;
		#[cfg(target_arch = "wasm32")]
		let project = web::open_project().await?;

		let viewport = Viewport::new(
			"ZenithEngineRenderer",
			window.inner_size().width,
			window.inner_size().height,
			window.clone(),
			PowerMode::Performance,
			false,
			&project,
		)
		.await?;

		let gamepads = match Gamepads::new() {
			Ok(gamepads) => Some(gamepads),
			Err(e) => {
				zenith_log::warn!("No gamepads: {e}");
				None
			}
		};

		let mut world = load_world(&project)?;
		let saved_overrides = load_input(&project, &mut world)?;

		Ok(Self {
			viewport,
			window,
			project,
			world,
			gamepads,
			last_frame: Instant::now(),
			saved_overrides,
			overrides_timer: 0.0,
		})
	}

	/// Writes the changes the player made to the controls, if there are new
	/// ones (or removes the file when there are none left). It is looked for
	/// every few seconds, since nothing says when the player changed a binding
	/// and a browser has no moment to save at the end.
	fn sync_overrides(&mut self) {
		let text = self.world.input().overrides_toml();
		if text == self.saved_overrides {
			return;
		}
		let written = zenith_filesystem::ZPath::new(INPUT_OVERRIDES).and_then(|path| {
			// NOTE: maybe here doesn't need to delete
			if text.is_empty() {
				self.project.delete(&path).map(|_| ())
			} else {
				self.project.write(&path, text.as_bytes())
			}
		});
		match written {
			Ok(()) => self.saved_overrides = text,
			Err(e) => zenith_log::warn!("Your controls could not be saved: {e}"),
		}
	}

	/// Runs one frame of the world: the gamepads are read, the systems run
	/// with the input of this frame, and the input is ready for the next.
	pub fn update(&mut self) {
		let now = Instant::now();
		let dt = now.duration_since(self.last_frame).as_secs_f32();
		self.last_frame = now;
		if let Some(gamepads) = &mut self.gamepads {
			gamepads.poll(&mut self.world.input_mut());
		}
		// Before the systems run: they end the frame, and the movement of the
		// mouse of this frame is forgotten with it.
		let camera = camera_input(&mut self.world.input_mut());
		self.viewport.update(dt, &camera);
		self.world.update_systems(dt);
		self.carry_out_cursor_request();
		self.overrides_timer += dt;
		if self.overrides_timer >= SAVE_OVERRIDES_EVERY {
			self.overrides_timer = 0.0;
			self.sync_overrides();
		}
	}

	/// What the game asked of the window (a hidden or locked cursor).
	fn carry_out_cursor_request(&mut self) {
		let mode = self.world.input_mut().take_cursor_mode();
		if let Some(mode) = mode
			&& let Err(e) = zenith_input::apply_cursor_mode(&self.window, mode)
		{
			zenith_log::warn!("{e}");
		}
	}

	/// Leaves nothing running behind the game: vibrations are stopped and the
	/// cursor is given back. Whatever way the game ends (the window is closed,
	/// Escape, Ctrl+C), this runs once, from `ApplicationHandler::exiting`.
	///
	/// The world is **not** saved here: the runtime plays a build, and what
	/// the player did belongs in a save slot, not in the world of the project.
	fn shutdown(&mut self) {
		let pads: Vec<usize> = self.world.input().list_gamepads().iter().map(|pad| pad.id).collect();
		for id in pads {
			self.world.input_mut().stop_rumble(id);
		}
		self.world.input_mut().set_cursor_mode(zenith_input::CursorMode::Normal);
		if let Some(gamepads) = &mut self.gamepads {
			gamepads.poll(&mut self.world.input_mut());
		}
		self.carry_out_cursor_request();
		self.sync_overrides();
		if let Err(e) = self.project.flush_data() {
			zenith_log::warn!("The data of the player could not be flushed: {e}");
		}
		zenith_log::info!("Shut down");
	}

	pub fn resize(&mut self, width: u32, height: u32) {
		self.viewport.resize(width, height);
	}

	/// ## Main Render function
	///
	/// ## Errors
	/// Return Error if Device lost.
	pub fn render(&mut self) -> ZResult<()> {
		self.window.request_redraw();
		self.viewport.render("Render Encoder", "Render Pass")?;
		Ok(())
	}
}

// wasm_bindgen(start) expands into top-level items assuming module scope;
// this has to sit outside any impl block, not as a State method.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn run_web() -> Result<(), wasm_bindgen::JsValue> {
	console_error_panic_hook::set_once();
	run().unwrap_throw();

	Ok(())
}

/// Run the `zenith_runtime`
///
/// # Errors
/// return error if run app is failed.
pub fn run() -> ZInternalResult<()> {
	#[cfg(target_arch = "wasm32")]
	{
		let _ = zenith_log::init_web();
	}

	let event_loop = EventLoop::with_user_event().build()?;
	#[cfg(not(target_arch = "wasm32"))]
	{
		native::quit_on_ctrl_c();
		let mut app = App::new();
		event_loop.run_app(&mut app)?;
	}
	#[cfg(target_arch = "wasm32")]
	{
		let app = App::new(&event_loop);
		event_loop.spawn_app(app);
	}

	Ok(())
}

pub struct App {
	#[cfg(target_arch = "wasm32")]
	proxy: Option<winit::event_loop::EventLoopProxy<State>>,
	state: Option<State>,
}

#[cfg(not(target_arch = "wasm32"))]
impl App {
	pub const fn new() -> Self {
		Self { state: None }
	}
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for App {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(target_arch = "wasm32")]
impl App {
	pub fn new(event_loop: &EventLoop<State>) -> Self {
		let proxy = Some(event_loop.create_proxy());
		Self { state: None, proxy }
	}
}

impl ApplicationHandler<State> for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		#[allow(unused_mut)]
		let mut window_attributes = Window::default_attributes()
			.with_title("Zenith Engine")
			.with_min_inner_size(winit::dpi::LogicalSize::new(320.0, 240.0))
			.with_visible(true)
			.with_transparent(true);

		#[cfg(not(target_arch = "wasm32"))]
		{
			window_attributes = window_attributes.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
		}

		#[cfg(target_os = "windows")]
		{
			use winit::platform::windows::{Color, WindowAttributesExtWindows};
			window_attributes = window_attributes
				.with_title_text_color(Color::from_rgb(84, 160, 255))
				.with_title_background_color(Some(Color::from_rgb(77, 77, 77)));
		}

		#[cfg(target_arch = "wasm32")]
		{
			use wasm_bindgen::JsCast;
			use winit::platform::web::WindowAttributesExtWebSys;

			const CANVAS_ID: &str = "zenith-canvas";

			let window = wgpu::web_sys::window().unwrap_throw();
			let document = window.document().unwrap_throw();
			let canvas = document.get_element_by_id(CANVAS_ID).unwrap_throw();
			let html_canvas_element = canvas.unchecked_into();

			window_attributes = window_attributes
				.with_canvas(Some(html_canvas_element))
				.with_prevent_default(false);
		}

		let window = Arc::new(match event_loop.create_window(window_attributes) {
			Ok(window) => window,
			Err(err) => {
				zenith_log::error!("Failed to create window: {}", err);
				std::process::exit(1);
			}
		});

		#[cfg(not(target_arch = "wasm32"))]
		{
			self.state = Some(match pollster::block_on(State::new(window)) {
				Ok(state) => state,
				Err(err) => {
					zenith_log::error!("Failed to create state: {}", err);
					std::process::exit(1);
				}
			});
		}

		#[cfg(target_arch = "wasm32")]
		{
			if let Some(proxy) = self.proxy.take() {
				wasm_bindgen_futures::spawn_local(async move {
					assert!(
						proxy
							.send_event(match State::new(window).await {
								Ok(state) => state,
								Err(e) => {
									zenith_log::error!("Unable to create canvas: {e}");
									std::process::exit(1);
								}
							})
							.is_ok()
					)
				})
			}
		}
	}

	#[allow(unused_mut)]
	fn user_event(&mut self, _event_loop: &ActiveEventLoop, mut event: State) {
		#[cfg(target_arch = "wasm32")]
		{
			event.window.request_redraw();
			event.resize(event.window.inner_size().width, event.window.inner_size().height);
		}
		self.state = Some(event);
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: winit::window::WindowId, event: WindowEvent) {
		let Some(state) = &mut self.state else { return };
		state.world.input_mut().handle_window_event(&event);

		match event {
			WindowEvent::CloseRequested => event_loop.exit(),
			WindowEvent::Resized(size) => state.resize(size.width, size.height),
			WindowEvent::RedrawRequested => {
				// Before the update: it ends the frame and forgets the press.
				if state.world.input().is_key_just_pressed(ZKeyCode::Escape) {
					event_loop.exit();
					return;
				}
				state.update();
				if let Err(e) = state.render() {
					zenith_log::error!("Cannot render: {e}");
				}
			}
			_ => {}
		}
	}

	fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
		if let Some(state) = &mut self.state {
			state.world.input_mut().handle_device_event(&event);
		}
	}

	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		// Ctrl+C is seen here, between events. A window that is hidden gets
		// no redraws, so the loop is woken up now and then to look.
		#[cfg(not(target_arch = "wasm32"))]
		{
			if native::quit_requested() {
				event_loop.exit();
				return;
			}
			event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
				std::time::Instant::now() + std::time::Duration::from_millis(100),
			));
		}
		#[cfg(target_arch = "wasm32")]
		let _ = event_loop;
	}

	fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
		if let Some(state) = &mut self.state {
			state.shutdown();
		}
	}
}
