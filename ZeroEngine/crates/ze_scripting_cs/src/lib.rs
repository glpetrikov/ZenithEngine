use std::{
	any::Any,
	cell::{Cell, RefCell},
	collections::{BTreeMap, HashMap, HashSet},
	env, fs,
	path::{Path, PathBuf},
	rc::Rc,
	sync::{Mutex, atomic::AtomicBool},
	time::{SystemTime, UNIX_EPOCH},
};

use fn_ptr::{WithAbi, abi::System as AbiSystem};
use netcorehost::{
	hostfxr::{
		AssemblyDelegateLoader, FnPtr, GetManagedFunctionError, Hostfxr, HostfxrContext, InitializedForRuntimeConfig,
	},
	pdcstr,
	pdcstring::{PdCStr, PdCString},
};
use schemars::schema_for;
use serde::{de, ser::SerializeMap};
use ze_assets::{AssetRef, CellId};
use ze_core::{Context, Result, Vec2, Vec3, anyhow};
use ze_ecs::{
	Component, Deserialize, EntitiesView, EntityId, JsonSchema, Scene, Serialize, System, registry::ComponentRegistry,
};
use ze_renderer::{Sprite, TextureSource};
use ze_ui::{UIBar, UIButton, UIImage, UIText};

const ASSEMBLY_PATH: &str = "Sandbox/assets/bin/Sandbox.dll";
const RUNTIME_CONFIG_PATH: &str = "Sandbox/assets/bin/Sandbox.runtimeconfig.json";
const SCRIPT_HOST_TYPE: &str = "ZeroEngine.ZEScript";

// Lifecycle methods carry entityId + classPath so multiple scripts on the same
// entity each look up their own C# instance slot (composite key).
pub type ScriptFn = <fn(u64, *const u8, i32) as WithAbi<AbiSystem>>::F;
pub type ScriptInitFn = <fn(u64, *const EngineAPI, *const u8, i32) as WithAbi<AbiSystem>>::F;
pub type ScriptEntityFn = <fn(u64, u64, *const u8, i32) as WithAbi<AbiSystem>>::F;
pub type ScriptMetadataFn = <fn(*const u8, i32, *mut u8, i32) -> i32 as WithAbi<AbiSystem>>::F;
pub type ScriptClassListFn = <fn(*mut u8, i32) -> i32 as WithAbi<AbiSystem>>::F;
pub type ScriptBridgeErrorFn = <fn(*mut u8, i32) -> i32 as WithAbi<AbiSystem>>::F;
pub type ScriptApplyFieldsFn = <fn(u64, *const u8, i32, *const u8, i32) -> i32 as WithAbi<AbiSystem>>::F;

#[repr(C)]
pub struct EngineAPI {
	pub is_key_pressed: extern "C" fn(i32) -> bool,
	pub is_key_just_pressed: extern "C" fn(i32) -> bool,
	pub is_key_released: extern "C" fn(i32) -> bool,
	pub is_key_just_released: extern "C" fn(i32) -> bool,
	pub is_mouse_button_pressed: extern "C" fn(i32) -> bool,
	pub is_mouse_button_just_pressed: extern "C" fn(i32) -> bool,
	pub is_mouse_button_released: extern "C" fn(i32) -> bool,
	pub is_mouse_button_just_released: extern "C" fn(i32) -> bool,
	pub get_mouse_position: extern "C" fn(*mut f32, *mut f32),
	pub get_mouse_delta: extern "C" fn(*mut f32, *mut f32),
	pub get_time_state_ptr: extern "C" fn() -> *const ScriptingTimeState,
	pub has_component: extern "C" fn(u64, u32) -> bool,
	pub get_position: extern "C" fn(u64, *mut f32, *mut f32),
	pub get_velocity: extern "C" fn(u64, *mut f32, *mut f32),
	pub add_2d_force: extern "C" fn(u64, f32, f32),
	pub add_2d_impulse: extern "C" fn(u64, f32, f32),
	pub play_audio: extern "C" fn(u64),
	pub stop_audio: extern "C" fn(u64),
	pub set_audio_volume: extern "C" fn(u64, f32),
	pub is_audio_playing: extern "C" fn(u64) -> bool,
	pub raycast_2d: extern "C" fn(f32, f32, f32, f32, f32, *mut f32, *mut f32, *mut f32, *mut f32, *mut u64) -> bool,
	pub get_sprite_texture_rotation_degrees: extern "C" fn(u64) -> f32,
	pub set_sprite_texture_rotation_degrees: extern "C" fn(u64, f32),
	pub set_button_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_button_hover_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_bar_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_bar_bg_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_text_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_bar_label: extern "C" fn(u64, *const u8, i32),
	pub set_button_pressed_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_button_text: extern "C" fn(u64, *const u8, i32),
	pub set_text_text: extern "C" fn(u64, *const u8, i32),
	pub set_text_font_size: extern "C" fn(u64, f32),
	pub is_button_clicked: extern "C" fn(u64) -> bool,
	pub set_image_color: extern "C" fn(u64, f32, f32, f32, f32),
	pub set_image_texture_path: extern "C" fn(u64, *const u8, i32),
	pub set_image_sheet_cell: extern "C" fn(u64, *const u8, i32, u32),
	pub scene_load: extern "C" fn(*const u8, i32),
	pub scene_load_main: extern "C" fn(),
	pub scene_reload: extern "C" fn(),
	pub log_info: extern "C" fn(*const u8, i32),
	pub log_warn: extern "C" fn(*const u8, i32),
	pub log_error: extern "C" fn(*const u8, i32),
	pub log_debug: extern "C" fn(*const u8, i32),
	pub quit_game: extern "C" fn(),
	pub set_animator_state: extern "C" fn(u64, *const u8, i32),
	pub set_velocity: extern "C" fn(u64, f32, f32),
	pub find_entity_by_name: extern "C" fn(*const u8, i32) -> u64,
	pub find_entity_by_tag: extern "C" fn(*const u8, i32) -> u64,
	pub find_entities_by_tag: extern "C" fn(*const u8, i32, *mut u64, i32) -> i32,
	pub find_entity_by_id: extern "C" fn(u32) -> u64,
	pub instantiate_entity: extern "C" fn(u64, f32, f32, f32, *mut u64) -> bool,
	pub destroy_entity: extern "C" fn(u64),
	pub add_component: extern "C" fn(u64, u32),
	pub remove_component: extern "C" fn(u64, u32),
	pub get_mouse_world_position: extern "C" fn(*mut f32, *mut f32),
	pub set_cursor_visible: extern "C" fn(i32),
	pub set_cursor_grab_mode: extern "C" fn(i32),
}

pub static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ScriptingTimeState {
	pub delta_time: f32,
	pub fixed_delta_time: f32,
	pub unscaled_delta_time: f32,
	pub time_scale: f32,
	pub time_since_startup: f64,
	pub unscaled_time_since_startup: f64,
	pub fixed_time: f64,
	pub frame_count: u64,
	pub fixed_frame_count: u64,
	pub is_fixed_update: u8,
}

impl ScriptingTimeState {
	const fn initial() -> Self {
		Self {
			delta_time: 0.0,
			fixed_delta_time: 0.0,
			unscaled_delta_time: 0.0,
			time_scale: 1.0,
			time_since_startup: 0.0,
			unscaled_time_since_startup: 0.0,
			fixed_time: 0.0,
			frame_count: 0,
			fixed_frame_count: 0,
			is_fixed_update: 0,
		}
	}
}

static ENGINE_API: EngineAPI = EngineAPI {
	is_key_pressed: api::is_key_pressed,
	is_key_just_pressed: api::is_key_just_pressed,
	is_key_released: api::is_key_released,
	is_key_just_released: api::is_key_just_released,
	is_mouse_button_pressed: api::is_mouse_button_pressed,
	is_mouse_button_just_pressed: api::is_mouse_button_just_pressed,
	is_mouse_button_released: api::is_mouse_button_released,
	is_mouse_button_just_released: api::is_mouse_button_just_released,
	get_mouse_position: api::get_mouse_position,
	get_mouse_delta: api::get_mouse_delta,
	get_time_state_ptr: api::get_time_state_ptr,
	has_component: api::has_component,
	get_position: api::get_position,
	get_velocity: api::get_velocity,
	add_2d_force: api::add_2d_force,
	add_2d_impulse: api::add_2d_impulse,
	play_audio: api::play_audio,
	stop_audio: api::stop_audio,
	set_audio_volume: api::set_audio_volume,
	is_audio_playing: api::is_audio_playing,
	raycast_2d: api::raycast_2d,
	get_sprite_texture_rotation_degrees: api::get_sprite_texture_rotation_degrees,
	set_sprite_texture_rotation_degrees: api::set_sprite_texture_rotation_degrees,
	set_button_color: api::set_button_color,
	set_button_hover_color: api::set_button_hover_color,
	set_bar_color: api::set_bar_color,
	set_bar_bg_color: api::set_bar_bg_color,
	set_text_color: api::set_text_color,
	set_bar_label: api::set_bar_label,
	set_button_pressed_color: api::set_button_pressed_color,
	set_button_text: api::set_button_text,
	set_text_text: api::set_text_text,
	set_text_font_size: api::set_text_font_size,
	is_button_clicked: api::is_button_clicked,
	set_image_color: api::set_image_color,
	set_image_texture_path: api::set_image_texture_path,
	set_image_sheet_cell: api::set_image_sheet_cell,
	scene_load: api::scene_load,
	scene_load_main: api::scene_load_main,
	scene_reload: api::scene_reload,
	log_info: api::log_info,
	log_warn: api::log_warn,
	log_error: api::log_error,
	log_debug: api::log_debug,
	quit_game: api::quit_game,
	set_animator_state: api::set_animator_state,
	set_velocity: api::set_velocity,
	find_entity_by_name: api::find_entity_by_name,
	find_entity_by_tag: api::find_entity_by_tag,
	find_entities_by_tag: api::find_entities_by_tag,
	find_entity_by_id: api::find_entity_by_id,
	instantiate_entity: api::instantiate_entity,
	destroy_entity: api::destroy_entity,
	add_component: api::add_component,
	remove_component: api::remove_component,
	get_mouse_world_position: api::get_mouse_world_position,
	set_cursor_visible: api::set_cursor_visible,
	set_cursor_grab_mode: api::set_cursor_grab_mode,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Script {
	pub path: String,
	#[serde(default = "default_enabled")]
	pub enabled: bool,
	#[serde(default)]
	pub fields: BTreeMap<String, ScriptFieldValue>,
}

/// Container component holding all script instances attached to an entity.
/// Replaces the single-slot `Script` component to allow multiple scripts per
/// entity.
#[derive(Component, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Scripts {
	pub list: Vec<Script>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptFieldMetadata {
	#[serde(alias = "Name")]
	pub name: String,
	#[serde(rename = "type", alias = "Type")]
	pub ty: String,
	#[serde(default = "default_script_field_source", alias = "Source")]
	pub source: ScriptFieldSource,
	#[serde(default, rename = "defaultValue", alias = "DefaultValue")]
	pub default_value: Option<ScriptFieldValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptFieldSource {
	Public,
	ZeroField,
}

impl ScriptFieldSource {
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Public => "public",
			Self::ZeroField => "ZeroField",
		}
	}
}

const fn default_script_field_source() -> ScriptFieldSource { ScriptFieldSource::Public }

#[derive(Debug, Clone, JsonSchema)]
#[schemars(untagged)]
pub enum ScriptFieldValue {
	Bool(bool),
	Int(i32),
	UInt(u32),
	Float(f32),
	String(String),
	#[schemars(with = "[f32; 2]")]
	Vec2(Vec2),
	#[schemars(with = "[f32; 3]")]
	Vec3(Vec3),
	Unsupported(serde_json::Value),
}

// Custom serde so Vec2/Vec3 use object format {"x":...,"y":...} instead of
// glam's default array format, matching C# System.Numerics expectations.
// Also accepts PascalCase {"X":...,"Y":...} (C# built-in Vector* converter)
// and legacy array format [x, y, ...] for scene backward compatibility.
impl Serialize for ScriptFieldValue {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Bool(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "bool")?;
				map.serialize_entry("value", v)?;
				map.end()
			}
			Self::Int(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "int")?;
				map.serialize_entry("value", v)?;
				map.end()
			}
			Self::UInt(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "uint")?;
				map.serialize_entry("value", v)?;
				map.end()
			}
			Self::Float(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "float")?;
				map.serialize_entry("value", v)?;
				map.end()
			}
			Self::String(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "string")?;
				map.serialize_entry("value", v)?;
				map.end()
			}
			Self::Vec2(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "vec2")?;
				map.serialize_entry("value", &serde_json::json!({"x": v.x, "y": v.y}))?;
				map.end()
			}
			Self::Vec3(v) => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("kind", "vec3")?;
				map.serialize_entry("value", &serde_json::json!({"x": v.x, "y": v.y, "z": v.z}))?;
				map.end()
			}
			Self::Unsupported(v) => v.serialize(serializer),
		}
	}
}

impl<'de> Deserialize<'de> for ScriptFieldValue {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = serde_json::Value::deserialize(deserializer)?;
		let obj = value
			.as_object()
			.ok_or_else(|| de::Error::custom("expected object for ScriptFieldValue"))?;
		let kind = obj.get("kind").and_then(|v| v.as_str());

		match kind {
			None => {
				// No 'kind' field — try to parse the object as a vector value.
				// This handles C# sending {} as a default value, which would
				// otherwise fail and cause the entire field to be skipped.
				parse_vec3_value(&value).map_or_else(
					|_| parse_vec2_value(&value).map_or_else(|_| Ok(Self::Unsupported(value)), |v| Ok(Self::Vec2(v))),
					|v| Ok(Self::Vec3(v)),
				)
			}
			Some("bool") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let b = val
					.as_bool()
					.ok_or_else(|| de::Error::custom("expected bool for ScriptFieldValue::Bool"))?;
				Ok(Self::Bool(b))
			}
			Some("int") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let i = val
					.as_i64()
					.ok_or_else(|| de::Error::custom("expected integer for ScriptFieldValue::Int"))?;
				Ok(Self::Int(i as i32))
			}
			Some("uint") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let u = val
					.as_u64()
					.ok_or_else(|| de::Error::custom("expected unsigned integer for ScriptFieldValue::UInt"))?;
				Ok(Self::UInt(u as u32))
			}
			Some("float") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let f = val
					.as_f64()
					.ok_or_else(|| de::Error::custom("expected float for ScriptFieldValue::Float"))?;
				Ok(Self::Float(f as f32))
			}
			Some("string") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let s = val
					.as_str()
					.ok_or_else(|| de::Error::custom("expected string for ScriptFieldValue::String"))?;
				Ok(Self::String(s.to_string()))
			}
			Some("vec2") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let v = parse_vec2_value(val).map_err(de::Error::custom)?;
				Ok(Self::Vec2(v))
			}
			Some("vec3") => {
				let val = obj
					.get("value")
					.ok_or_else(|| de::Error::custom("missing 'value' field in ScriptFieldValue"))?;
				let v = parse_vec3_value(val).map_err(de::Error::custom)?;
				Ok(Self::Vec3(v))
			}
			Some(_) => Ok(Self::Unsupported(value)),
		}
	}
}

fn parse_vec2_value(val: &serde_json::Value) -> Result<Vec2, String> {
	match val {
		serde_json::Value::Array(arr) if arr.len() >= 2 => {
			let x = arr[0].as_f64().ok_or_else(|| "vec2 array[0] not a float".to_string())? as f32;
			let y = arr[1].as_f64().ok_or_else(|| "vec2 array[1] not a float".to_string())? as f32;
			Ok(Vec2::new(x, y))
		}
		serde_json::Value::Object(map) => {
			let x = extract_vec_f32_or_default(map, &["x", "X"]);
			let y = extract_vec_f32_or_default(map, &["y", "Y"]);
			Ok(Vec2::new(x, y))
		}
		_ => Err("vec2 must be an array [x, y] or object {{\"x\":..., \"y\":...}}".to_string()),
	}
}

fn parse_vec3_value(val: &serde_json::Value) -> Result<Vec3, String> {
	match val {
		serde_json::Value::Array(arr) if arr.len() >= 3 => {
			let x = arr[0].as_f64().ok_or_else(|| "vec3 array[0] not a float".to_string())? as f32;
			let y = arr[1].as_f64().ok_or_else(|| "vec3 array[1] not a float".to_string())? as f32;
			let z = arr[2].as_f64().ok_or_else(|| "vec3 array[2] not a float".to_string())? as f32;
			Ok(Vec3::new(x, y, z))
		}
		serde_json::Value::Object(map) => {
			let x = extract_vec_f32_or_default(map, &["x", "X"]);
			let y = extract_vec_f32_or_default(map, &["y", "Y"]);
			let z = extract_vec_f32_or_default(map, &["z", "Z"]);
			Ok(Vec3::new(x, y, z))
		}
		_ => Err("vec3 must be an array [x, y, z] or object {{\"x\":..., \"y\":..., \"z\":...}}".to_string()),
	}
}

fn extract_vec_f32_or_default(map: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> f32 {
	for key in keys {
		if let Some(value) = map.get(*key)
			&& let Some(f) = value.as_f64()
		{
			return f as f32;
		}
	}
	0.0
}

impl Default for Script {
	fn default() -> Self {
		Self {
			path: "Script".to_string(),
			enabled: true,
			fields: BTreeMap::new(),
		}
	}
}

pub fn register_scripting_components(registry: &mut ComponentRegistry) {
	registry.register::<Scripts>("ze.scripting.scripts");

	let old_schema = serde_json::to_value(schema_for!(Script)).expect("Script schema serialization failed");
	registry.register_custom(
		"ze.scripting.script",
		old_schema,
		Box::new(|_, _| None),
		Box::new(|entity, world, value| {
			let script: Script = serde_json::from_value(value)?;
			if let Ok(mut existing) = world.get::<&mut Scripts>(entity) {
				existing.list.push(script);
			} else {
				world.add_component(entity, (Scripts { list: vec![script] },));
			}
			Ok(())
		}),
	);
}

static RUNTIME_CONTEXT: Mutex<Option<HostfxrContext<InitializedForRuntimeConfig>>> = Mutex::new(None);

fn global_runtime_context(
	runtime_config_path: &PdCStr,
) -> Result<std::sync::MutexGuard<'static, Option<HostfxrContext<InitializedForRuntimeConfig>>>> {
	let mut guard = RUNTIME_CONTEXT.lock().expect("C# runtime context mutex was poisoned");
	if guard.is_none() {
		let hostfxr = load_hostfxr().context("failed to load .NET hostfxr")?;
		let context = hostfxr
			.initialize_for_runtime_config(runtime_config_path)
			.context("failed to initialize .NET runtime for Scripts")?;
		*guard = Some(context);
	}
	Ok(guard)
}

pub struct ScriptingEngine {
	assembly: Option<ScriptAssembly>,
}

struct ScriptAssembly {
	loader: AssemblyDelegateLoader,
	assembly_name: String,
	_load_dir: Option<ScriptLoadDir>,
	logged_script_metadata: RefCell<HashSet<String>>,
}

impl ScriptingEngine {
	pub fn new() -> Result<Self> { Self::from_paths(ASSEMBLY_PATH.into(), RUNTIME_CONFIG_PATH.into()) }

	pub fn from_paths(assembly_path: PathBuf, runtime_config_path: PathBuf) -> Result<Self> {
		let runtime_config_path = normalize_path(runtime_config_path)?;
		let prepared_runtime_config_path = PdCString::from_os_str(&runtime_config_path).with_context(|| {
			format!(
				"failed to encode C# runtime config path {}",
				runtime_config_path.display()
			)
		})?;

		let guard = global_runtime_context(&prepared_runtime_config_path)?;
		let context = guard
			.as_ref()
			.expect("C# runtime should be initialized after global_runtime_context succeeds");
		let assembly = ScriptAssembly::from_paths(context, assembly_path, runtime_config_path)?;
		drop(guard);
		assembly.validate_bridge()?;

		Ok(Self {
			assembly: Some(assembly),
		})
	}

	pub fn load_script(&self, class_path: &str) -> Result<ScriptLifecycle> { self.assembly()?.load_script(class_path) }

	pub fn describe_script_fields(&self, class_path: &str) -> Result<Option<Vec<ScriptFieldMetadata>>> {
		self.assembly()?.describe_script_fields(class_path)
	}

	pub fn list_script_classes(&self) -> Result<Vec<String>> { self.assembly()?.list_script_classes() }

	pub fn apply_script_fields(
		&self,
		entity: EntityId,
		class_path: &str,
		fields: &BTreeMap<String, ScriptFieldValue>,
	) -> Result<()> {
		self.assembly()?.apply_script_fields(entity, class_path, fields)
	}

	fn ensure_assembly_loaded(&self) -> Result<()> {
		self.assembly()?;
		Ok(())
	}

	fn reload_from_paths(&mut self, assembly_path: PathBuf, runtime_config_path: PathBuf) -> Result<()> {
		// Save the old runtime context so we can restore it if the new
		// assembly fails to load.  The context must be taken *before*
		// calling from_paths so that global_runtime_context creates a
		// fresh HostfxrContext (and therefore a fresh AssemblyLoadContext)
		// for the newly-built DLL.
		let old_context = RUNTIME_CONTEXT
			.lock()
			.expect("C# runtime context mutex was poisoned")
			.take();

		// Load the new assembly — this creates a fresh runtime context
		// and loads the rebuilt Scripts.dll under a new ALC so the new
		// types (e.g. new script fields) are visible to reflection.
		let new_engine = match Self::from_paths(assembly_path, runtime_config_path) {
			Ok(engine) => engine,
			Err(error) => {
				// Restore the old context so that the previously-loaded
				// assembly remains usable.  Without this, the old
				// ScriptAssembly's delegate loader would reference a
				// destroyed HostfxrContext, leading to use-after-free
				// on the next DescribeScriptFields call.
				*RUNTIME_CONTEXT.lock().expect("C# runtime context mutex was poisoned") = old_context;
				return Err(error);
			}
		};

		// Old context is deliberately discarded here.  The old
		// ScriptAssembly is still owned by self.assembly, but its
		// HostfxrContext is no longer reachable — it will be cleaned up
		// when the old ScriptAssembly is dropped below.
		self.assembly = None;
		self.assembly = new_engine.assembly;
		Ok(())
	}

	fn assembly(&self) -> Result<&ScriptAssembly> {
		self.assembly
			.as_ref()
			.ok_or_else(|| anyhow!("C# script assembly is not loaded"))
	}
}

impl ScriptAssembly {
	fn from_paths(
		context: &HostfxrContext<InitializedForRuntimeConfig>,
		assembly_path: PathBuf,
		runtime_config_path: PathBuf,
	) -> Result<Self> {
		let assembly_path = normalize_path(assembly_path)?;
		let runtime_config_path = normalize_path(runtime_config_path)?;
		let assembly_name = assembly_name_from_path(&assembly_path)?;
		let prepared = prepare_reloadable_assembly(&assembly_path, &runtime_config_path)?;

		ze_log::debug!(
			"loading C# assembly `{assembly_name}` from original `{}` → shadow `{}`",
			assembly_path.display(),
			prepared.assembly_path.display()
		);

		let assembly_path = PdCString::from_os_str(&prepared.assembly_path)
			.with_context(|| format!("failed to encode C# assembly path {}", prepared.assembly_path.display()))?;
		let loader = context
			.get_delegate_loader_for_assembly(assembly_path)
			.context("failed to load C# assembly")?;

		Ok(Self {
			loader,
			assembly_name,
			_load_dir: prepared.load_dir,
			logged_script_metadata: RefCell::new(HashSet::new()),
		})
	}

	fn validate_bridge(&self) -> Result<()> {
		let type_name = PdCString::from_os_str(format!("{SCRIPT_HOST_TYPE}, {}", self.assembly_name))
			.with_context(|| format!("failed to encode C# script host type `{SCRIPT_HOST_TYPE}`"))?;
		let get_error = load_optional_function::<ScriptBridgeErrorFn>(
			&self.loader,
			type_name.as_ref(),
			SCRIPT_HOST_TYPE,
			"NativeGetLastScriptBridgeError",
			pdcstr!("NativeGetLastScriptBridgeError"),
		)?;

		if get_error.is_some() {
			Ok(())
		} else {
			Err(anyhow!(
				"C# script bridge `{SCRIPT_HOST_TYPE}.NativeGetLastScriptBridgeError` was not found in script assembly `{}`; rebuild scripts and ensure ZeroEngine/api/cs/ZEScript.cs is included in the scripts project",
				self.assembly_name
			))
		}
	}

	fn load_script(&self, class_path: &str) -> Result<ScriptLifecycle> {
		ScriptLifecycle::load(&self.loader, &self.assembly_name, class_path)
	}

	fn describe_script_fields(&self, class_path: &str) -> Result<Option<Vec<ScriptFieldMetadata>>> {
		let type_name = PdCString::from_os_str(format!("{SCRIPT_HOST_TYPE}, {}", self.assembly_name))
			.with_context(|| format!("failed to encode C# script host type `{SCRIPT_HOST_TYPE}`"))?;
		let describe_fields = load_optional_function::<ScriptMetadataFn>(
			&self.loader,
			type_name.as_ref(),
			class_path,
			"NativeDescribeScriptFields",
			pdcstr!("NativeDescribeScriptFields"),
		)?;

		let Some(describe_fields) = describe_fields else {
			return Err(anyhow!(
				"C# script metadata bridge `{SCRIPT_HOST_TYPE}.NativeDescribeScriptFields` was not found in script assembly `{}`; rebuild scripts and ensure ZeroEngine/api/cs/ZEScript.cs is included in the scripts project",
				self.assembly_name
			));
		};

		let class_path_len = i32::try_from(class_path.len()).unwrap_or(i32::MAX);
		let required_size = describe_fields(class_path.as_ptr(), class_path_len, std::ptr::null_mut(), 0);
		if required_size < 0 {
			let error = self.script_field_metadata_error(type_name.as_ref(), class_path);
			return Err(anyhow!("{error}"));
		}

		let mut buffer = vec![0u8; required_size as usize];
		let written = describe_fields(
			class_path.as_ptr(),
			class_path_len,
			buffer.as_mut_ptr(),
			i32::try_from(buffer.len()).unwrap_or(i32::MAX),
		);
		if written < 0 {
			let error = self.script_field_metadata_error(type_name.as_ref(), class_path);
			return Err(anyhow!("{error}"));
		}

		let raw_json = String::from_utf8_lossy(&buffer[..written as usize]).into_owned();
		ze_log::debug!("raw C# script field metadata for `{class_path}`:\n{}", raw_json);

		let fields = Self::parse_script_field_metadata(&raw_json, class_path)?;
		self.log_script_field_metadata(class_path, &fields);
		Ok(Some(fields))
	}

	/// Parse script field metadata with per-field fault tolerance.
	/// If a specific field's default value fails to deserialize, it is logged
	/// and replaced with a metadata entry carrying no default value, so that
	/// one broken field does not drop the entire metadata sheet.
	fn parse_script_field_metadata(raw_json: &str, class_path: &str) -> Result<Vec<ScriptFieldMetadata>> {
		let entries: Vec<serde_json::Value> = serde_json::from_str(raw_json)
			.with_context(|| format!("failed to parse C# script field metadata JSON for `{class_path}`"))?;

		let mut fields = Vec::with_capacity(entries.len());
		for entry in entries {
			match serde_json::from_value::<ScriptFieldMetadata>(entry.clone()) {
				Ok(field) => fields.push(field),
				Err(err) => {
					let raw = serde_json::to_string(&entry).unwrap_or_else(|_| "<unprintable>".to_string());
					ze_log::error!(
						"skipping script field for `{class_path}` — deserialization error: {err}\n  raw entry: {raw}"
					);
					// Fallback: try to at least extract name + type so the
					// field shows up in the inspector (without a default value).
					let name = entry
						.get("name")
						.or_else(|| entry.get("Name"))
						.and_then(|v| v.as_str())
						.unwrap_or("<unknown>")
						.to_string();
					let ty = entry
						.get("type")
						.or_else(|| entry.get("Type"))
						.and_then(|v| v.as_str())
						.unwrap_or("<unknown>")
						.to_string();
					let source = entry
						.get("source")
						.or_else(|| entry.get("Source"))
						.and_then(|v| v.as_str())
						.unwrap_or("public");
					let source = match source {
						"zeroField" | "ZeroField" => ScriptFieldSource::ZeroField,
						_ => ScriptFieldSource::Public,
					};
					ze_log::error!(
						"broken script field `{name}` (`{ty}`, source: {source:?}) in `{class_path}` — \
						 default value format not recognised; field will be shown without a default"
					);
					fields.push(ScriptFieldMetadata {
						name,
						ty,
						source,
						default_value: None,
					});
				}
			}
		}

		Ok(fields)
	}

	fn list_script_classes(&self) -> Result<Vec<String>> {
		let type_name = PdCString::from_os_str(format!("{SCRIPT_HOST_TYPE}, {}", self.assembly_name))
			.with_context(|| format!("failed to encode C# script host type `{SCRIPT_HOST_TYPE}`"))?;
		let list_scripts = load_optional_function::<ScriptClassListFn>(
			&self.loader,
			type_name.as_ref(),
			SCRIPT_HOST_TYPE,
			"NativeListScripts",
			pdcstr!("NativeListScripts"),
		)?;

		let Some(list_scripts) = list_scripts else {
			return Err(anyhow!(
				"C# script metadata bridge `{SCRIPT_HOST_TYPE}.NativeListScripts` was not found in script assembly `{}`; rebuild scripts and ensure ZeroEngine/api/cs/ZEScript.cs is included in the scripts project",
				self.assembly_name
			));
		};

		let required_size = list_scripts(std::ptr::null_mut(), 0);
		if required_size < 0 {
			let error = self.script_field_metadata_error(type_name.as_ref(), SCRIPT_HOST_TYPE);
			return Err(anyhow!("{error}"));
		}

		let mut buffer = vec![0u8; required_size as usize];
		let written = list_scripts(buffer.as_mut_ptr(), i32::try_from(buffer.len()).unwrap_or(i32::MAX));
		if written < 0 {
			let error = self.script_field_metadata_error(type_name.as_ref(), SCRIPT_HOST_TYPE);
			return Err(anyhow!("{error}"));
		}

		let raw_json = String::from_utf8_lossy(&buffer[..written as usize]).into_owned();
		ze_log::debug!("C# script classes raw JSON ({} bytes): {raw_json}", written);
		let classes: Vec<String> = serde_json::from_slice(&buffer[..written as usize])
			.with_context(|| "failed to parse C# script class metadata".to_string())?;
		ze_log::debug!("found {} C# script class(es): {classes:?}", classes.len());
		Ok(classes)
	}

	fn log_script_field_metadata(&self, class_path: &str, fields: &[ScriptFieldMetadata]) {
		if !self.logged_script_metadata.borrow_mut().insert(class_path.to_string()) {
			return;
		}

		if fields.is_empty() {
			return;
		}

		let _field_summary = fields
			.iter()
			.map(|field| {
				let default_value = field
					.default_value
					.as_ref()
					.map(|value| format!(" = {}", format_script_field_value(value)))
					.unwrap_or_default();
				format!(
					"{}: {} ({}){}",
					field.name,
					field.ty,
					field.source.as_str(),
					default_value
				)
			})
			.collect::<Vec<_>>()
			.join(", ");
	}

	fn script_field_metadata_error(&self, type_name: &PdCStr, class_path: &str) -> String {
		let fallback = format!(
			"failed to describe C# script fields for `{class_path}` in assembly `{}`",
			self.assembly_name
		);

		let Some(bridge_error) = self.last_script_bridge_error(type_name, class_path) else {
			return format!("{fallback}; the C# metadata bridge returned failure without details");
		};

		format!("{fallback}: {bridge_error}")
	}

	fn last_script_bridge_error(&self, type_name: &PdCStr, class_path: &str) -> Option<String> {
		let get_error = match load_optional_function::<ScriptBridgeErrorFn>(
			&self.loader,
			type_name,
			class_path,
			"NativeGetLastScriptBridgeError",
			pdcstr!("NativeGetLastScriptBridgeError"),
		) {
			Ok(Some(function)) => function,
			Ok(None) => return None,
			Err(error) => {
				ze_log::warn!("failed to load C# script bridge error function for `{class_path}`: {error:?}");
				return None;
			}
		};

		let required_size = get_error(std::ptr::null_mut(), 0);
		if required_size <= 0 {
			return None;
		}

		let mut buffer = vec![0u8; required_size as usize];
		let written = get_error(buffer.as_mut_ptr(), i32::try_from(buffer.len()).unwrap_or(i32::MAX));
		if written <= 0 {
			return None;
		}

		let message_length = usize::min(written as usize, buffer.len());
		let message = String::from_utf8_lossy(&buffer[..message_length]).into_owned();
		if message.trim().is_empty() { None } else { Some(message) }
	}

	fn apply_script_fields(
		&self,
		entity: EntityId,
		class_path: &str,
		fields: &BTreeMap<String, ScriptFieldValue>,
	) -> Result<()> {
		if fields.is_empty() {
			return Ok(());
		}

		let type_name = PdCString::from_os_str(format!("{SCRIPT_HOST_TYPE}, {}", self.assembly_name))
			.with_context(|| format!("failed to encode C# script host type `{SCRIPT_HOST_TYPE}`"))?;
		let apply_fields = load_optional_function::<ScriptApplyFieldsFn>(
			&self.loader,
			type_name.as_ref(),
			SCRIPT_HOST_TYPE,
			"NativeApplyScriptFields",
			pdcstr!("NativeApplyScriptFields"),
		)?;

		let Some(apply_fields) = apply_fields else {
			return Ok(());
		};

		let json = serde_json::to_string(fields).with_context(|| {
			format!(
				"failed to serialize C# script field values for entity {}",
				entity.index()
			)
		})?;
		let entity_id = entity_id_to_script_arg(entity);
		let class_path_len = i32::try_from(class_path.len()).unwrap_or(i32::MAX);
		let result = apply_fields(
			entity_id,
			class_path.as_ptr(),
			class_path_len,
			json.as_ptr(),
			i32::try_from(json.len()).unwrap_or(i32::MAX),
		);

		if result < 0 {
			let fallback = format!(
				"failed to apply C# script field values for `{class_path}` on entity {}",
				entity.index()
			);
			let error = self
				.last_script_bridge_error(type_name.as_ref(), class_path)
				.map_or_else(
					|| format!("{fallback}; the C# field apply bridge returned failure without details"),
					|error| format!("{fallback}: {error}"),
				);
			return Err(anyhow!("{error}"));
		}

		Ok(())
	}
}

fn format_script_field_value(value: &ScriptFieldValue) -> String {
	match value {
		ScriptFieldValue::Bool(value) => value.to_string(),
		ScriptFieldValue::Int(value) => value.to_string(),
		ScriptFieldValue::UInt(value) => value.to_string(),
		ScriptFieldValue::Float(value) => value.to_string(),
		ScriptFieldValue::String(value) => format!("{value:?}"),
		ScriptFieldValue::Vec2(value) => format!("{}, {}", value.x, value.y),
		ScriptFieldValue::Vec3(value) => format!("{}, {}, {}", value.x, value.y, value.z),
		ScriptFieldValue::Unsupported(value) => value.to_string(),
	}
}

pub struct ScriptLifecycle {
	pub on_engine_init: Option<ScriptInitFn>,
	pub on_create: Option<ScriptFn>,
	pub on_start: Option<ScriptFn>,
	pub on_destroy: Option<ScriptFn>,
	pub on_update: Option<ScriptFn>,
	pub on_fixed_update: Option<ScriptFn>,
	pub on_enable: Option<ScriptFn>,
	pub on_disable: Option<ScriptFn>,
	pub on_contact_enter: Option<ScriptEntityFn>,
	pub on_contact_stay: Option<ScriptEntityFn>,
	pub on_contact_exit: Option<ScriptEntityFn>,
	pub on_sensor_enter: Option<ScriptEntityFn>,
	pub on_sensor_stay: Option<ScriptEntityFn>,
	pub on_sensor_exit: Option<ScriptEntityFn>,
}

impl ScriptLifecycle {
	fn load(loader: &AssemblyDelegateLoader, script_assembly_name: &str, class_path: &str) -> Result<Self> {
		let type_name = PdCString::from_os_str(format!("{SCRIPT_HOST_TYPE}, {script_assembly_name}"))
			.with_context(|| format!("failed to encode C# script host type `{SCRIPT_HOST_TYPE}`"))?;

		Ok(Self {
			on_engine_init: load_optional_function::<fn(u64, *const EngineAPI, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnEngineInit",
				pdcstr!("NativeOnEngineInit"),
			)?,
			on_create: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnCreate",
				pdcstr!("NativeOnCreate"),
			)?,
			on_start: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnStart",
				pdcstr!("NativeOnStart"),
			)?,
			on_destroy: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnDestroy",
				pdcstr!("NativeOnDestroy"),
			)?,
			on_update: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnUpdate",
				pdcstr!("NativeOnUpdate"),
			)?,
			on_fixed_update: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnFixedUpdate",
				pdcstr!("NativeOnFixedUpdate"),
			)?,
			on_enable: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnEnable",
				pdcstr!("NativeOnEnable"),
			)?,
			on_disable: load_optional_function::<fn(u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnDisable",
				pdcstr!("NativeOnDisable"),
			)?,
			on_contact_enter: load_optional_function::<fn(u64, u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnContactEnter",
				pdcstr!("NativeOnContactEnter"),
			)?,
			on_contact_stay: load_optional_function::<fn(u64, u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnContactStay",
				pdcstr!("NativeOnContactStay"),
			)?,
			on_contact_exit: load_optional_function::<fn(u64, u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnContactExit",
				pdcstr!("NativeOnContactExit"),
			)?,
			on_sensor_enter: load_optional_function::<fn(u64, u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnSensorEnter",
				pdcstr!("NativeOnSensorEnter"),
			)?,
			on_sensor_stay: load_optional_function::<fn(u64, u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnSensorStay",
				pdcstr!("NativeOnSensorStay"),
			)?,
			on_sensor_exit: load_optional_function::<fn(u64, u64, *const u8, i32)>(
				loader,
				type_name.as_ref(),
				class_path,
				"NativeOnSensorExit",
				pdcstr!("NativeOnSensorExit"),
			)?,
		})
	}

	pub fn on_engine_init(&self, entity: EntityId, class_path: &str) {
		if let Some(on_engine_init) = self.on_engine_init {
			let class_path_len = i32::try_from(class_path.len()).unwrap_or(i32::MAX);
			on_engine_init(
				entity_id_to_script_arg(entity),
				&raw const ENGINE_API,
				class_path.as_ptr(),
				class_path_len,
			);
		}
	}

	pub fn on_create(&self, entity: EntityId, path: &str) {
		if let Some(on_create) = self.on_create {
			on_create(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_start(&self, entity: EntityId, path: &str) {
		if let Some(on_start) = self.on_start {
			on_start(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_destroy(&self, entity: EntityId, path: &str) {
		if let Some(on_destroy) = self.on_destroy {
			on_destroy(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_update(&self, entity: EntityId, path: &str) {
		if let Some(on_update) = self.on_update {
			on_update(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_fixed_update(&self, entity: EntityId, path: &str) {
		if let Some(on_fixed_update) = self.on_fixed_update {
			on_fixed_update(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_enable(&self, entity: EntityId, path: &str) {
		if let Some(on_enable) = self.on_enable {
			on_enable(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_disable(&self, entity: EntityId, path: &str) {
		if let Some(on_disable) = self.on_disable {
			on_disable(
				entity_id_to_script_arg(entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_contact_enter(&self, entity: EntityId, other_entity: EntityId, path: &str) {
		if let Some(on_contact_enter) = self.on_contact_enter {
			on_contact_enter(
				entity_id_to_script_arg(entity),
				entity_id_to_script_arg(other_entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_contact_stay(&self, entity: EntityId, other_entity: EntityId, path: &str) {
		if let Some(on_contact_stay) = self.on_contact_stay {
			on_contact_stay(
				entity_id_to_script_arg(entity),
				entity_id_to_script_arg(other_entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_contact_exit(&self, entity: EntityId, other_entity: EntityId, path: &str) {
		if let Some(on_contact_exit) = self.on_contact_exit {
			on_contact_exit(
				entity_id_to_script_arg(entity),
				entity_id_to_script_arg(other_entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_sensor_enter(&self, entity: EntityId, other_entity: EntityId, path: &str) {
		if let Some(on_sensor_enter) = self.on_sensor_enter {
			on_sensor_enter(
				entity_id_to_script_arg(entity),
				entity_id_to_script_arg(other_entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_sensor_stay(&self, entity: EntityId, other_entity: EntityId, path: &str) {
		if let Some(on_sensor_stay) = self.on_sensor_stay {
			on_sensor_stay(
				entity_id_to_script_arg(entity),
				entity_id_to_script_arg(other_entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}

	pub fn on_sensor_exit(&self, entity: EntityId, other_entity: EntityId, path: &str) {
		if let Some(on_sensor_exit) = self.on_sensor_exit {
			on_sensor_exit(
				entity_id_to_script_arg(entity),
				entity_id_to_script_arg(other_entity),
				path.as_ptr(),
				i32::try_from(path.len()).unwrap_or(i32::MAX),
			);
		}
	}
}

pub fn entity_id_to_script_arg(entity: EntityId) -> u64 {
	((u64::from(entity.r#gen())) << 32) | (entity.index() & 0xffff_ffff)
}

pub const fn script_arg_to_entity_id(entity: u64) -> EntityId {
	let index = entity & 0xffff_ffff;
	let generation = (entity >> 32) as u16;
	EntityId::new_from_index_and_gen(index, generation)
}

/// A single 2D raycast result, mirrored 1:1 by the out-params `raycast_2d`
/// writes for C#.
#[derive(Debug, Clone, Copy)]
pub struct RaycastHit2D {
	pub point: Vec2,
	pub normal: Vec2,
	pub entity: EntityId,
}

/// Bridges raycast queries from scripts into `ze_physics`'s rapier2d world.
///
/// `ze_physics` owns rapier2d's query pipeline and can't be depended on from
/// here directly -- it already depends on `ze_scripting_cs`, so a reverse
/// dependency would be circular. Instead `PhysicsSystem` registers a plain fn
/// pointer + opaque context pointer for the duration of each `fixed_update`
/// call it drives, the same "expose latest physics state to scripts" idea
/// as the velocity cache, but for a query that takes arbitrary parameters
/// instead of being keyed by entity.
pub type RaycastQueryFn = fn(*const (), Vec2, Vec2, f32) -> Option<RaycastHit2D>;

thread_local! {
	static RAYCAST_PROVIDER: Cell<Option<(*const (), RaycastQueryFn)>> = const { Cell::new(None) };
}

/// # Safety
/// `context` must stay valid for as long as the provider is registered. Callers
/// must pair this with [`clear_raycast_provider`] before `context`'s pointee
/// can be moved, reset, or dropped.
pub unsafe fn set_raycast_provider(context: *const (), query: RaycastQueryFn) {
	RAYCAST_PROVIDER.with(|cell| cell.set(Some((context, query))));
}

pub fn clear_raycast_provider() { RAYCAST_PROVIDER.with(|cell| cell.set(None)); }

#[derive(Debug, Clone, Copy)]
pub enum ScriptingApiCommand {
	Add2DForce { entity: EntityId, x: f32, y: f32 },
	Add2DImpulse { entity: EntityId, x: f32, y: f32 },
	SetVelocity { entity: EntityId, x: f32, y: f32 },
}

/// How the OS cursor should be constrained to the window.
///
/// Mirrors C#'s `Input.CursorGrabMode`. Drained via [`drain_cursor_commands`]
/// and applied by whichever shell owns the winit window (the standalone app or
/// the editor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptingCursorGrabMode {
	None,
	Confined,
	Locked,
}

/// Cursor visibility/grab requests queued by C# scripts, drained by the window
/// owner.
///
/// Modelled as discrete commands (rather than polled state) so a script that
/// never touches the cursor leaves the shell's own defaults alone.
#[derive(Debug, Clone, Copy)]
pub enum CursorApiCommand {
	SetVisible(bool),
	SetGrabMode(ScriptingCursorGrabMode),
}

/// Fire-and-forget audio commands queued by C# scripts.
///
/// Drained by `ze_audio`'s `AudioSystem`, the same relationship
/// `ScriptingApiCommand` has with `ze_physics`. Every variant is entity-keyed
/// -- the entity's own `AudioSource` component (`ze_ecs::components`) is what
/// says which clip/volume/looping to use, so unlike the earlier
/// single-global-music-channel design there's no ambiguity about "which" sound
/// a command targets.
#[derive(Debug, Clone, Copy)]
pub enum AudioApiCommand {
	Play { entity: EntityId },
	Stop { entity: EntityId },
	SetVolume { entity: EntityId, volume: f32 },
}

/// Fire-and-forget animation-state-switch commands queued by C# scripts,
/// drained by `ze_animation`'s `AnimationSystem` (same relationship
/// `AudioApiCommand` has with `ze_audio`'s `AudioSystem`).
#[derive(Debug, Clone)]
pub enum AnimatorApiCommand {
	SetState { entity: EntityId, state: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptingSceneLoadCommand {
	Load { name: String },
	LoadMain,
	Reload,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone)]
enum ScriptingSceneCommand {
	SetSpriteTextureRotationDegrees {
		entity: EntityId,
		degrees: f32,
	},
	SetButtonColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetButtonHoverColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetBarColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetBarBgColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetTextColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetBarLabel {
		entity: EntityId,
		text: String,
	},
	SetButtonPressedColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetButtonText {
		entity: EntityId,
		text: String,
	},
	SetTextText {
		entity: EntityId,
		text: String,
	},
	SetTextFontSize {
		entity: EntityId,
		font_size: f32,
	},
	SetImageColor {
		entity: EntityId,
		r: f32,
		g: f32,
		b: f32,
		a: f32,
	},
	SetImageTexturePath {
		entity: EntityId,
		path: String,
	},
	SetImageSheetCell {
		entity: EntityId,
		sheet_path: String,
		cell_id: u32,
	},
}

pub fn refresh_scripting_api_velocity_cache(velocities: impl IntoIterator<Item = (EntityId, f32, f32)>) {
	api::refresh_velocity_cache(velocities);
}

pub fn drain_scripting_api_commands() -> Vec<ScriptingApiCommand> { api::drain_commands() }

pub fn drain_audio_api_commands() -> Vec<AudioApiCommand> { api::drain_audio_commands() }

pub fn drain_animator_api_commands() -> Vec<AnimatorApiCommand> { api::drain_animator_commands() }

pub fn refresh_scripting_api_audio_playing_cache(playing: impl IntoIterator<Item = (EntityId, bool)>) {
	api::refresh_audio_playing_cache(playing);
}

pub fn drain_scripting_scene_load_commands() -> Vec<ScriptingSceneLoadCommand> { api::drain_scene_load_commands() }

pub fn drain_cursor_commands() -> Vec<CursorApiCommand> { api::drain_cursor_commands() }

fn drain_scripting_scene_commands() -> Vec<ScriptingSceneCommand> { api::drain_scene_commands() }

pub fn sync_scene_script_fields(scene: &mut Scene, runtime: &ScriptingRuntimeHandle) -> Result<usize> {
	runtime.sync_scene_script_fields(scene)
}

/// RAII guard that makes `scene` reachable from the C# FFI callbacks for as
/// long as it is held, and disconnects it on drop.
///
/// While C# scripts run they are driven synchronously and single-threaded from
/// Rust, and the outer `&mut Scene` borrow is parked on the stack (not touched)
/// for the duration of each callback -- the same reentrancy story as the
/// raycast provider. Construct one around a block that will call into C# (a
/// script tick or a collision dispatch) so entity-manipulation FFI
/// (`Instantiate`, `Destroy`, `AddComponent`, ...) can operate on the real
/// scene.
pub struct SceneProviderGuard {
	_private: (),
}

impl SceneProviderGuard {
	pub fn new(scene: &mut Scene) -> Self {
		api::set_scene_provider(std::ptr::from_mut(scene));
		Self { _private: () }
	}
}

impl Drop for SceneProviderGuard {
	fn drop(&mut self) { api::clear_scene_provider(); }
}

struct ScriptInstance {
	path: String,
	lifecycle: ScriptLifecycle,
	started: bool,
	enabled: bool,
}

struct ScriptingRuntime {
	engine: Option<ScriptingEngine>,
	instances: HashMap<EntityId, Vec<ScriptInstance>>,
	last_engine_init_error: Option<String>,
}

impl ScriptingRuntime {
	fn new() -> Self { Self::new_with_paths(ASSEMBLY_PATH.into(), RUNTIME_CONFIG_PATH.into()) }

	fn new_with_paths(assembly_path: PathBuf, runtime_config_path: PathBuf) -> Self {
		let (engine, last_engine_init_error) = match ScriptingEngine::from_paths(assembly_path, runtime_config_path) {
			Ok(engine) => (Some(engine), None),
			Err(error) => {
				let message = format!("{error:?}");
				ze_log::error!("failed to initialize C# scripting engine: {message}");
				(None, Some(message))
			}
		};
		Self {
			engine,
			instances: HashMap::new(),
			last_engine_init_error,
		}
	}

	fn sync_instances(&mut self, scripts: &[(EntityId, Script)]) {
		// Group scripts by entity for efficient comparison.
		let mut entity_scripts: HashMap<EntityId, Vec<&Script>> = HashMap::new();
		for (entity, script) in scripts {
			entity_scripts.entry(*entity).or_default().push(script);
		}

		// Remove entities that no longer have any scripts.
		let removed: Vec<EntityId> = self
			.instances
			.keys()
			.copied()
			.filter(|e| !entity_scripts.contains_key(e))
			.collect();
		for entity in removed {
			self.destroy_all_instances(entity);
		}

		// Collect new instances to create (separate from the mutable borrow of
		// instances).
		let mut to_create: Vec<(EntityId, &Script)> = Vec::new();

		// Sync each entity's script instances.
		for (entity, current_scripts) in &entity_scripts {
			let instances = self.instances.entry(*entity).or_default();

			// Remove instances whose class path is no longer in current scripts.
			instances.retain(|instance| {
				if current_scripts.iter().any(|s| s.path == instance.path) {
					true
				} else {
					if instance.enabled {
						instance.lifecycle.on_disable(*entity, &instance.path);
					}
					instance.lifecycle.on_destroy(*entity, &instance.path);
					false
				}
			});

			for script in current_scripts {
				if let Some(instance) = instances.iter_mut().find(|i| i.path == script.path) {
					match (script.enabled, instance.enabled) {
						(true, false) => {
							instance.lifecycle.on_enable(*entity, &instance.path);
							instance.enabled = true;
						}
						(false, true) => {
							instance.lifecycle.on_disable(*entity, &instance.path);
							instance.enabled = false;
						}
						_ => {}
					}
				} else if script.enabled {
					to_create.push((*entity, *script));
				}
			}
		}

		// Create new instances outside the mutable borrow of self.instances.
		for (entity, script) in to_create {
			match self.create_instance(entity, script) {
				Ok(instance) => {
					self.instances.entry(entity).or_default().push(instance);
				}
				Err(error) => {
					ze_log::error!(
						"failed to create C# script instance `{}` for entity {}: {error:?}",
						script.path,
						entity.index()
					);
				}
			}
		}
	}

	fn create_instance(&self, entity: EntityId, script: &Script) -> Result<ScriptInstance> {
		let engine = self.engine()?;
		let lifecycle = engine.load_script(&script.path)?;
		lifecycle.on_engine_init(entity, &script.path);
		if let Err(error) = engine.apply_script_fields(entity, &script.path, &script.fields) {
			ze_log::warn!(
				"failed to apply C# script field values for `{}` on entity {}: {error:?}",
				script.path,
				entity.index()
			);
		}
		lifecycle.on_create(entity, &script.path);
		lifecycle.on_enable(entity, &script.path);

		Ok(ScriptInstance {
			path: script.path.clone(),
			lifecycle,
			started: false,
			enabled: true,
		})
	}

	fn destroy_all_instances(&mut self, entity: EntityId) {
		let Some(instances) = self.instances.remove(&entity) else {
			return;
		};

		for instance in instances {
			if instance.enabled {
				instance.lifecycle.on_disable(entity, &instance.path);
			}
			instance.lifecycle.on_destroy(entity, &instance.path);
		}
	}

	fn update(&mut self, scene: &mut Scene, dt: f32) -> Result<()> {
		self.ensure_engine_loaded()?;
		api::begin_update(dt);
		// Expose the live scene to FFI callbacks (Instantiate/Destroy/AddComponent/
		// GetMouseWorldPosition) for the duration of this tick; cleared on drop even
		// on the early-return paths below.
		let _scene_guard = SceneProviderGuard::new(scene);
		api::refresh_scene_cache(scene);
		if self.needs_scene_script_field_sync(scene) {
			self.sync_scene_script_fields(scene)?;
		}
		let scripts = scene_scripts(scene);
		self.sync_instances(&scripts);

		for (entity, instances) in &mut self.instances {
			for instance in instances.iter_mut().filter(|i| i.enabled) {
				if !instance.started {
					let path = instance.path.clone();
					instance.lifecycle.on_start(*entity, &path);
					instance.started = true;
				}

				instance.lifecycle.on_update(*entity, &instance.path);
			}
		}

		apply_scripting_scene_commands(scene);
		api::refresh_scene_cache(scene);
		Ok(())
	}

	fn fixed_update(&mut self, scene: &mut Scene, dt: f32) -> Result<()> {
		self.ensure_engine_loaded()?;
		api::begin_fixed_update(dt);
		let _scene_guard = SceneProviderGuard::new(scene);
		api::refresh_scene_cache(scene);
		if self.needs_scene_script_field_sync(scene) {
			self.sync_scene_script_fields(scene)?;
		}
		let scripts = scene_scripts(scene);
		self.sync_instances(&scripts);

		for (entity, instances) in &self.instances {
			for instance in instances.iter().filter(|i| i.enabled) {
				instance.lifecycle.on_fixed_update(*entity, &instance.path);
			}
		}

		apply_scripting_scene_commands(scene);
		api::refresh_scene_cache(scene);
		Ok(())
	}

	fn for_each_enabled_instance(&self, entity: EntityId, mut f: impl FnMut(&ScriptInstance)) {
		if let Some(instances) = self.instances.get(&entity) {
			for instance in instances.iter().filter(|i| i.enabled) {
				f(instance);
			}
		}
	}

	fn destroy_all(&mut self) {
		for (entity, instances) in self.instances.drain() {
			for instance in instances {
				if instance.enabled {
					instance.lifecycle.on_disable(entity, &instance.path);
				}
				instance.lifecycle.on_destroy(entity, &instance.path);
			}
		}
	}

	fn on_contact_enter(&self, entity: EntityId, other_entity: EntityId) {
		self.for_each_enabled_instance(entity, |instance| {
			instance
				.lifecycle
				.on_contact_enter(entity, other_entity, &instance.path);
		});
	}

	fn on_contact_stay(&self, entity: EntityId, other_entity: EntityId) {
		self.for_each_enabled_instance(entity, |instance| {
			instance.lifecycle.on_contact_stay(entity, other_entity, &instance.path);
		});
	}

	fn on_contact_exit(&self, entity: EntityId, other_entity: EntityId) {
		self.for_each_enabled_instance(entity, |instance| {
			instance.lifecycle.on_contact_exit(entity, other_entity, &instance.path);
		});
	}

	fn on_sensor_enter(&self, entity: EntityId, other_entity: EntityId) {
		self.for_each_enabled_instance(entity, |instance| {
			instance.lifecycle.on_sensor_enter(entity, other_entity, &instance.path);
		});
	}

	fn on_sensor_stay(&self, entity: EntityId, other_entity: EntityId) {
		self.for_each_enabled_instance(entity, |instance| {
			instance.lifecycle.on_sensor_stay(entity, other_entity, &instance.path);
		});
	}

	fn on_sensor_exit(&self, entity: EntityId, other_entity: EntityId) {
		self.for_each_enabled_instance(entity, |instance| {
			instance.lifecycle.on_sensor_exit(entity, other_entity, &instance.path);
		});
	}

	fn sync_scene_script_fields(&self, scene: &mut Scene) -> Result<usize> {
		sync_scene_script_fields_with(scene, |class_path| self.describe_script_fields(class_path))
	}

	fn needs_scene_script_field_sync(&self, scene: &Scene) -> bool {
		scene_scripts(scene).into_iter().any(|(entity, script)| {
			!script.fields.is_empty()
				&& self
					.instances
					.get(&entity)
					.is_none_or(|instances| !instances.iter().any(|i| i.path == script.path))
		})
	}

	fn describe_script_fields(&self, class_path: &str) -> Result<Option<Vec<ScriptFieldMetadata>>> {
		self.engine()?.describe_script_fields(class_path)
	}

	fn list_script_classes(&self) -> Result<Vec<String>> { self.engine()?.list_script_classes() }

	fn reload_from_paths(&mut self, assembly_path: PathBuf, runtime_config_path: PathBuf) -> Result<()> {
		self.destroy_all();
		self.instances.clear();

		if self.engine.is_none() {
			// Engine was never initialized (e.g., initial build failed or
			// the DLL was not available at scene load time).  Try to create
			// a fresh one now so a rebuilt or newly-available Scripts.dll
			// can be picked up without restarting the editor.
			return match ScriptingEngine::from_paths(assembly_path, runtime_config_path) {
				Ok(new_engine) => {
					self.engine = Some(new_engine);
					self.last_engine_init_error = None;
					Ok(())
				}
				Err(error) => {
					let message = format!("{error:?}");
					ze_log::error!("failed to initialize C# scripting engine: {message}");
					self.last_engine_init_error = Some(message);
					Err(error)
				}
			};
		}

		let engine = self.engine.as_mut().expect("engine was just verified to be Some");
		match engine.reload_from_paths(assembly_path, runtime_config_path) {
			Ok(()) => {
				self.last_engine_init_error = None;
				Ok(())
			}
			Err(error) => {
				let message = format!("{error:?}");
				ze_log::error!("failed to reload C# script assembly: {message}");
				self.last_engine_init_error = Some(message);
				Err(error)
			}
		}
	}

	fn ensure_engine_loaded(&self) -> Result<()> { self.engine()?.ensure_assembly_loaded() }

	fn engine(&self) -> Result<&ScriptingEngine> { self.engine.as_ref().map_or_else(|| self.engine_not_loaded(), Ok) }

	fn engine_not_loaded<T>(&self) -> Result<T> {
		self.last_engine_init_error.as_ref().map_or_else(
			|| Err(anyhow!("C# scripting engine is not loaded")),
			|error| {
				Err(anyhow!(
					"C# scripting engine is not loaded; previous initialization failed: {error}"
				))
			},
		)
	}
}

#[derive(Clone)]
pub struct ScriptingRuntimeHandle(Rc<RefCell<ScriptingRuntime>>);

impl Default for ScriptingRuntimeHandle {
	fn default() -> Self { Self::new() }
}

impl ScriptingRuntimeHandle {
	pub fn new() -> Self { Self(Rc::new(RefCell::new(ScriptingRuntime::new()))) }

	pub fn new_with_paths(assembly_path: PathBuf, runtime_config_path: PathBuf) -> Self {
		Self(Rc::new(RefCell::new(ScriptingRuntime::new_with_paths(
			assembly_path,
			runtime_config_path,
		))))
	}

	pub fn update(&self, scene: &mut Scene, dt: f32) -> Result<()> { self.0.borrow_mut().update(scene, dt) }

	pub fn fixed_update(&self, scene: &mut Scene, dt: f32) -> Result<()> { self.0.borrow_mut().fixed_update(scene, dt) }

	pub fn on_contact_enter(&self, entity: EntityId, other_entity: EntityId) {
		self.0.borrow().on_contact_enter(entity, other_entity);
	}

	pub fn on_contact_stay(&self, entity: EntityId, other_entity: EntityId) {
		self.0.borrow().on_contact_stay(entity, other_entity);
	}

	pub fn on_contact_exit(&self, entity: EntityId, other_entity: EntityId) {
		self.0.borrow().on_contact_exit(entity, other_entity);
	}

	pub fn on_sensor_enter(&self, entity: EntityId, other_entity: EntityId) {
		self.0.borrow().on_sensor_enter(entity, other_entity);
	}

	pub fn on_sensor_stay(&self, entity: EntityId, other_entity: EntityId) {
		self.0.borrow().on_sensor_stay(entity, other_entity);
	}

	pub fn on_sensor_exit(&self, entity: EntityId, other_entity: EntityId) {
		self.0.borrow().on_sensor_exit(entity, other_entity);
	}

	pub fn describe_script_fields(&self, class_path: &str) -> Result<Option<Vec<ScriptFieldMetadata>>> {
		self.0.borrow().describe_script_fields(class_path)
	}

	pub fn list_script_classes(&self) -> Result<Vec<String>> { self.0.borrow().list_script_classes() }

	pub fn sync_scene_script_fields(&self, scene: &mut Scene) -> Result<usize> {
		self.0.borrow().sync_scene_script_fields(scene)
	}

	fn destroy_all(&self) { self.0.borrow_mut().destroy_all(); }

	fn reload_from_paths(&self, assembly_path: PathBuf, runtime_config_path: PathBuf) -> Result<()> {
		self.0
			.borrow_mut()
			.reload_from_paths(assembly_path, runtime_config_path)
	}
}

pub struct ScriptingSystem {
	runtime: ScriptingRuntimeHandle,
}

impl Default for ScriptingSystem {
	fn default() -> Self { Self::new() }
}

impl ScriptingSystem {
	pub fn new() -> Self {
		Self {
			runtime: ScriptingRuntimeHandle::new(),
		}
	}

	pub fn from_paths(assembly_path: PathBuf, runtime_config_path: PathBuf) -> Self {
		Self {
			runtime: ScriptingRuntimeHandle::new_with_paths(assembly_path, runtime_config_path),
		}
	}

	pub const fn from_runtime(runtime: ScriptingRuntimeHandle) -> Self { Self { runtime } }

	pub fn runtime(&self) -> ScriptingRuntimeHandle { self.runtime.clone() }

	pub fn reset(&mut self) { self.runtime.destroy_all(); }

	pub fn reload_from_paths(&mut self, assembly_path: PathBuf, runtime_config_path: PathBuf) -> Result<()> {
		self.runtime.reload_from_paths(assembly_path, runtime_config_path)
	}
}

impl System for ScriptingSystem {
	fn name(&self) -> &'static str { "ScriptingSystem" }

	fn update(&mut self, scene: &mut Scene, dt: f32) -> Result<()> { self.runtime.update(scene, dt) }

	fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

impl Drop for ScriptingSystem {
	fn drop(&mut self) { self.runtime.destroy_all(); }
}

#[allow(clippy::too_many_lines)]
fn apply_scripting_scene_commands(scene: &mut Scene) {
	for command in drain_scripting_scene_commands() {
		match command {
			ScriptingSceneCommand::SetSpriteTextureRotationDegrees { entity, degrees } => {
				match scene.world_mut().get::<&mut Sprite>(entity) {
					Ok(mut sprite) => {
						sprite.settings.texture_rotation_degrees = degrees;
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply sprite rotation command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetButtonColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIButton>(entity) {
					Ok(mut button) => {
						button.color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply button color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetButtonHoverColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIButton>(entity) {
					Ok(mut button) => {
						button.hover_color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply button hover color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetBarColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIBar>(entity) {
					Ok(mut bar) => {
						bar.color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply bar color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetBarBgColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIBar>(entity) {
					Ok(mut bar) => {
						bar.bg_color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply bar bg color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetTextColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIText>(entity) {
					Ok(mut text) => {
						text.color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply text color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetBarLabel { entity, text } => match scene.world_mut().get::<&mut UIBar>(entity) {
				Ok(mut bar) => {
					bar.text = if text.is_empty() { None } else { Some(text) };
				}
				Err(error) => {
					ze_log::warn!(
						"failed to apply bar label command for entity {}: {error:?}",
						entity.index()
					);
				}
			},
			ScriptingSceneCommand::SetButtonPressedColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIButton>(entity) {
					Ok(mut button) => {
						button.pressed_color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply button pressed color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetButtonText { entity, text } => {
				match scene.world_mut().get::<&mut UIButton>(entity) {
					Ok(mut button) => {
						button.text = text;
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply button text command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetTextText { entity, text } => match scene.world_mut().get::<&mut UIText>(entity) {
				Ok(mut ui_text) => {
					ui_text.text = text;
				}
				Err(error) => {
					ze_log::warn!(
						"failed to apply text content command for entity {}: {error:?}",
						entity.index()
					);
				}
			},
			ScriptingSceneCommand::SetTextFontSize { entity, font_size } => {
				match scene.world_mut().get::<&mut UIText>(entity) {
					Ok(mut ui_text) => {
						ui_text.font_size = font_size;
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply text font size command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetImageColor { entity, r, g, b, a } => {
				match scene.world_mut().get::<&mut UIImage>(entity) {
					Ok(mut image) => {
						image.color = [r, g, b, a];
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply image color command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetImageTexturePath { entity, path } => {
				match scene.world_mut().get::<&mut UIImage>(entity) {
					Ok(mut image) => {
						image.texture = TextureSource::File(AssetRef::game(path));
					}
					Err(error) => {
						ze_log::warn!(
							"failed to apply image texture path command for entity {}: {error:?}",
							entity.index()
						);
					}
				}
			}
			ScriptingSceneCommand::SetImageSheetCell {
				entity,
				sheet_path,
				cell_id,
			} => match scene.world_mut().get::<&mut UIImage>(entity) {
				Ok(mut image) => {
					image.texture = TextureSource::SheetCell {
						sheet_path,
						cell_id: CellId(cell_id),
					};
				}
				Err(error) => {
					ze_log::warn!(
						"failed to apply image sheet cell command for entity {}: {error:?}",
						entity.index()
					);
				}
			},
		}
	}
}

fn scene_scripts(scene: &Scene) -> Vec<(EntityId, Script)> {
	let world = scene.world();
	let mut scripts = Vec::new();

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			let Ok(scripts_component) = world.get::<&Scripts>(entity) else {
				continue;
			};

			for script in &scripts_component.list {
				scripts.push((entity, script.clone()));
			}
		}
	});

	scripts
}

fn sync_scene_script_fields_with(
	scene: &mut Scene,
	mut describe_fields: impl FnMut(&str) -> Result<Option<Vec<ScriptFieldMetadata>>>,
) -> Result<usize> {
	let scripts = scene_scripts(scene);
	let mut metadata_by_class = HashMap::<String, Option<HashSet<String>>>::new();
	let mut pruned = 0usize;

	for (entity, script) in scripts {
		if script.fields.is_empty() {
			continue;
		}

		if !metadata_by_class.contains_key(&script.path) {
			let current_fields = describe_fields(&script.path)?
				.map(|fields| fields.into_iter().map(|field| field.name).collect::<HashSet<_>>());
			metadata_by_class.insert(script.path.clone(), current_fields);
		}

		let Some(Some(current_fields)) = metadata_by_class.get(&script.path) else {
			continue;
		};

		let original_count = script.fields.len();
		if script.fields.keys().all(|field| current_fields.contains(field)) {
			continue;
		}

		if let Ok(mut current_scripts) = scene.world_mut().get::<&mut Scripts>(entity)
			&& let Some(current_script) = current_scripts.list.iter_mut().find(|s| s.path == script.path)
		{
			current_script
				.fields
				.retain(|field, _value| current_fields.contains(field));
			pruned += original_count.saturating_sub(current_script.fields.len());
		}
	}

	Ok(pruned)
}

fn load_optional_function<F>(
	loader: &AssemblyDelegateLoader,
	type_name: &PdCStr,
	class_path: &str,
	label: &'static str,
	method_name: &PdCStr,
) -> Result<Option<<F as WithAbi<AbiSystem>>::F>>
where
	F: FnPtr + WithAbi<AbiSystem>,
	<F as WithAbi<AbiSystem>>::F: Copy,
{
	match loader.get_function_with_unmanaged_callers_only::<F>(type_name, method_name) {
		Ok(function) => Ok(Some(*function)),
		Err(GetManagedFunctionError::TypeOrMethodNotFound) => Ok(None),
		Err(error) => Err(anyhow!(
			"failed to load C# script method `{class_path}.{label}`: {error}"
		)),
	}
}

fn normalize_path(path: PathBuf) -> Result<PathBuf> {
	if path.exists() {
		return Ok(path);
	}

	let repo_path = workspace_root()?.join(&path);
	if repo_path.exists() {
		return Ok(repo_path);
	}

	Err(anyhow!("path does not exist: {}", path.display()))
}

struct PreparedScriptAssembly {
	assembly_path: PathBuf,
	load_dir: Option<ScriptLoadDir>,
}

struct ScriptLoadDir(PathBuf);

impl Drop for ScriptLoadDir {
	fn drop(&mut self) {
		if let Err(error) = fs::remove_dir_all(&self.0) {
			ze_log::warn!(
				"failed to remove staged C# script assembly directory {}: {error}",
				self.0.display()
			);
		}
	}
}

fn prepare_reloadable_assembly(assembly_path: &Path, runtime_config_path: &Path) -> Result<PreparedScriptAssembly> {
	let load_dir = create_script_load_dir(assembly_path)?;
	let shadow_assembly_path = copy_to_dir(assembly_path, &load_dir)?;
	copy_to_dir(runtime_config_path, &load_dir)?;
	copy_optional_sidecar(assembly_path, &load_dir, "deps.json")?;
	copy_optional_sidecar(assembly_path, &load_dir, "pdb")?;

	Ok(PreparedScriptAssembly {
		assembly_path: shadow_assembly_path,
		load_dir: Some(ScriptLoadDir(load_dir)),
	})
}

fn create_script_load_dir(assembly_path: &Path) -> Result<PathBuf> {
	let assembly_name = assembly_path
		.file_stem()
		.and_then(|name| name.to_str())
		.unwrap_or("Scripts");
	let unique = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |duration| duration.as_nanos());
	let root = env::temp_dir().join("zeroengine-script-runtime");
	fs::create_dir_all(&root)
		.with_context(|| format!("failed to create script assembly load root {}", root.display()))?;

	for attempt in 0..16 {
		let directory = root.join(format!("{assembly_name}-{}-{unique}-{attempt}", std::process::id()));
		match fs::create_dir(&directory) {
			Ok(()) => return Ok(directory),
			Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
			Err(error) => {
				return Err(error).with_context(|| {
					format!(
						"failed to create script assembly load directory {}",
						directory.display()
					)
				});
			}
		}
	}

	Err(anyhow!(
		"failed to create a unique script assembly load directory under {}",
		root.display()
	))
}

fn copy_to_dir(source: &Path, target_dir: &Path) -> Result<PathBuf> {
	let file_name = source
		.file_name()
		.ok_or_else(|| anyhow!("script assembly sidecar path has no file name: {}", source.display()))?;
	let target = target_dir.join(file_name);
	fs::copy(source, &target)
		.with_context(|| format!("failed to stage script file {} for hot reload", source.display()))?;
	Ok(target)
}

fn copy_optional_sidecar(assembly_path: &Path, target_dir: &Path, extension: &str) -> Result<()> {
	let sidecar = assembly_path.with_extension(extension);
	if sidecar.exists() {
		copy_to_dir(&sidecar, target_dir)?;
	}
	Ok(())
}

fn assembly_name_from_path(path: &Path) -> Result<String> {
	path.file_stem()
		.and_then(|name| name.to_str())
		.map(ToOwned::to_owned)
		.ok_or_else(|| anyhow!("failed to determine C# assembly name from {}", path.display()))
}

fn workspace_root() -> Result<PathBuf> { ze_core::resolve_engine_root() }

fn load_hostfxr() -> Result<Hostfxr> {
	// Stage 1: Bundled deployment (Dist build priority)
	if let Some(bundled_path) = bundled_hostfxr_path() {
		ze_log::debug!("Using bundled hostfxr at: {}", bundled_path.display());
		return Hostfxr::load_from_path(&bundled_path)
			.with_context(|| format!("Failed to load bundled hostfxr from {}", bundled_path.display()));
	}
	ze_log::debug!("Bundled dotnet runtime not found next to executable");

	// Stage 2: Runtime system discovery (Replaces nethost compile-time discovery)
	if let Some(detected_path) = find_system_hostfxr_path() {
		ze_log::debug!("Successfully located system hostfxr at: {}", detected_path.display());
		return Hostfxr::load_from_path(&detected_path)
			.with_context(|| format!("Failed to load system hostfxr from {}", detected_path.display()));
	}
	ze_log::debug!("System hostfxr discovery failed");

	// Stage 3: Manual path enumeration fallback (CI / Edge cases)
	ze_log::debug!("Attempting manual path enumeration fallback...");
	let hostfxr_path = find_hostfxr_path_fallback().context("Failed to locate libhostfxr via any available method")?;

	Hostfxr::load_from_path(&hostfxr_path)
		.with_context(|| format!("Failed to load hostfxr from fallback path: {}", hostfxr_path.display()))
}

/// Helper to scan well-known system candidates at runtime.
fn find_system_hostfxr_path() -> Option<PathBuf> {
	for root in dotnet_root_candidates() {
		if let Some(path) = latest_hostfxr_path_in(&root) {
			return Some(path);
		}
	}
	None
}

/// Check for a bundled dotnet runtime next to the executable (dist build
/// scenario).
fn bundled_hostfxr_path() -> Option<PathBuf> {
	let exe_dir = env::current_exe().ok()?.parent()?.to_path_buf();
	let bundled = exe_dir.join("dotnet");
	if bundled.join("host/fxr").exists() {
		return latest_hostfxr_path_in(&bundled);
	}
	None
}

/// Manual fallback: enumerate well-known dotnet root directories.
fn find_hostfxr_path_fallback() -> Option<PathBuf> {
	for root in dotnet_root_candidates() {
		if let Some(path) = latest_hostfxr_path_in(&root) {
			return Some(path);
		}
	}
	None
}

/// Known dotnet install locations.
fn dotnet_root_candidates() -> Vec<PathBuf> {
	let mut roots = Vec::new();

	if let Some(root) = env::var_os("DOTNET_ROOT") {
		roots.push(PathBuf::from(root));
	}
	if let Some(home) = env::var_os("HOME") {
		roots.push(PathBuf::from(home).join(".dotnet"));
	}

	roots.push(PathBuf::from("C:/Program Files/dotnet"));
	roots.push(PathBuf::from("/usr/share/dotnet"));
	roots.push(PathBuf::from("/usr/local/share/dotnet"));
	roots
}

/// Return the path to the newest hostfxr library under a dotnet root.
fn latest_hostfxr_path_in(dotnet_root: &Path) -> Option<PathBuf> {
	let fxr_root = dotnet_root.join("host/fxr");
	let mut versions = fs::read_dir(fxr_root)
		.ok()?
		.filter_map(std::result::Result::ok)
		.filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
		.map(|entry| entry.path())
		.collect::<Vec<_>>();

	versions.sort();
	versions.reverse();

	let lib_name = if cfg!(windows) {
		"hostfxr.dll"
	} else if cfg!(target_os = "macos") {
		"libhostfxr.dylib"
	} else {
		"libhostfxr.so"
	};

	versions.into_iter().map(|v| v.join(lib_name)).find(|p| p.exists())
}

const fn default_enabled() -> bool { true }

mod api {
	use std::{
		cell::{Cell, RefCell, UnsafeCell},
		collections::{HashMap, HashSet},
	};

	use ze_core::Vec2;
	use ze_ecs::{
		ActiveCameraView, Animator, AudioSource, Children, Collider, EntitiesView, EntityId, Inactive, Name, Parent,
		PhysicsSettings, RigidBody, Scene, Tag, Transform, shipyard::UniqueView,
	};
	use ze_input::{Input, ZKeyCode, ZMouseCode};
	use ze_renderer::{Camera, Sprite};
	use ze_ui::{UIBar, UIButton, UIImage, UIText};

	use crate::{
		AnimatorApiCommand, AudioApiCommand, CursorApiCommand, RAYCAST_PROVIDER, ScriptingApiCommand,
		ScriptingCursorGrabMode, ScriptingSceneCommand, ScriptingSceneLoadCommand, ScriptingTimeState, Scripts,
		entity_id_to_script_arg, script_arg_to_entity_id,
	};

	/// Sentinel returned by the entity-lookup FFI functions when no entity
	/// matches; mirrors `Entity.Null` on the C# side.
	const INVALID_ENTITY: u64 = u64::MAX;

	#[derive(Default)]
	struct ApiState {
		commands: Vec<ScriptingApiCommand>,
		audio_commands: Vec<AudioApiCommand>,
		animator_commands: Vec<AnimatorApiCommand>,
		scene_load_commands: Vec<ScriptingSceneLoadCommand>,
		scene_commands: Vec<ScriptingSceneCommand>,
		cursor_commands: Vec<CursorApiCommand>,
		components: HashSet<(EntityId, ComponentKind)>,
		transform_positions: HashMap<EntityId, (f32, f32)>,
		velocities: HashMap<EntityId, (f32, f32)>,
		sprite_texture_rotations: HashMap<EntityId, f32>,
		button_clicked: HashMap<EntityId, bool>,
		audio_playing: HashMap<EntityId, bool>,
		names: HashMap<String, EntityId>,
		tags: HashMap<String, Vec<EntityId>>,
		entities_by_index: HashMap<u32, EntityId>,
	}

	struct TimeStateCell(UnsafeCell<ScriptingTimeState>);

	unsafe impl Sync for TimeStateCell {}

	static TIME_STATE: TimeStateCell = TimeStateCell(UnsafeCell::new(ScriptingTimeState::initial()));

	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
	enum ComponentKind {
		Name = 1,
		Tag = 2,
		Transform = 3,
		Parent = 4,
		Children = 5,
		Inactive = 6,
		Rigidbody = 7,
		PhysicsSettings = 8,
		Collider = 9,
		Sprite = 10,
		Camera = 11,
		Script = 12,
		UIButton = 13,
		UIBar = 14,
		UIText = 15,
		Audio = 16,
		Animator = 17,
		UIImage = 18,
	}

	thread_local! {
		static API_STATE: RefCell<ApiState> = RefCell::new(ApiState::default());
		// Raw pointer to the scene currently being ticked, set by `SceneProviderGuard`
		// for the span of each C# callback so entity-manipulation FFI can reach the
		// live world. `None` outside a script tick.
		static SCENE_PROVIDER: Cell<Option<*mut Scene>> = const { Cell::new(None) };
	}

	pub fn set_scene_provider(scene: *mut Scene) { SCENE_PROVIDER.with(|cell| cell.set(Some(scene))); }

	pub fn clear_scene_provider() { SCENE_PROVIDER.with(|cell| cell.set(None)); }

	/// Runs `f` against the scene currently being ticked, if any.
	///
	/// # Safety contract
	/// Relies on the invariant upheld by `SceneProviderGuard`: the pointer is
	/// only ever set to a `&mut Scene` that stays valid and untouched by outer
	/// Rust code for the duration of the synchronous C# callback that reaches
	/// this function.
	fn with_scene<R>(f: impl FnOnce(&mut Scene) -> R) -> Option<R> {
		SCENE_PROVIDER.with(Cell::get).map(|ptr| f(unsafe { &mut *ptr }))
	}

	pub fn drain_commands() -> Vec<ScriptingApiCommand> {
		API_STATE.with(|state| state.borrow_mut().commands.drain(..).collect())
	}

	pub fn drain_cursor_commands() -> Vec<CursorApiCommand> {
		API_STATE.with(|state| state.borrow_mut().cursor_commands.drain(..).collect())
	}

	pub fn drain_audio_commands() -> Vec<AudioApiCommand> {
		API_STATE.with(|state| state.borrow_mut().audio_commands.drain(..).collect())
	}

	pub fn drain_animator_commands() -> Vec<AnimatorApiCommand> {
		API_STATE.with(|state| state.borrow_mut().animator_commands.drain(..).collect())
	}

	pub fn drain_scene_load_commands() -> Vec<ScriptingSceneLoadCommand> {
		API_STATE.with(|state| state.borrow_mut().scene_load_commands.drain(..).collect())
	}

	pub fn drain_scene_commands() -> Vec<ScriptingSceneCommand> {
		API_STATE.with(|state| state.borrow_mut().scene_commands.drain(..).collect())
	}

	fn time_state_mut() -> &'static mut ScriptingTimeState { unsafe { &mut *TIME_STATE.0.get() } }

	pub fn begin_update(dt: f32) {
		let dt = dt.max(0.0);
		let time = time_state_mut();
		let time_scale = time.time_scale;
		time.delta_time = dt * time_scale;
		time.unscaled_delta_time = dt;
		time.time_since_startup += f64::from(time.delta_time);
		time.unscaled_time_since_startup += f64::from(dt);
		time.frame_count = time.frame_count.saturating_add(1);
		time.is_fixed_update = 0;
	}

	pub fn begin_fixed_update(dt: f32) {
		let dt = dt.max(0.0);
		let time = time_state_mut();
		let time_scale = time.time_scale;
		time.delta_time = dt * time_scale;
		time.fixed_delta_time = dt;
		time.unscaled_delta_time = dt;
		time.fixed_time += f64::from(dt);
		time.fixed_frame_count = time.fixed_frame_count.saturating_add(1);
		time.is_fixed_update = 1;
	}

	pub fn refresh_scene_cache(scene: &Scene) {
		let world = scene.world();
		let mut components = HashSet::new();
		let mut transform_positions = HashMap::new();
		let mut sprite_texture_rotations = HashMap::new();
		let mut button_clicked = HashMap::new();
		let mut names = HashMap::new();
		let mut tags: HashMap<String, Vec<EntityId>> = HashMap::new();
		let mut entities_by_index = HashMap::new();

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				entities_by_index.insert((entity.index() & 0xffff_ffff) as u32, entity);
				if let Ok(name) = world.get::<&Name>(entity) {
					components.insert((entity, ComponentKind::Name));
					// First writer wins so `Find` is stable when names collide.
					names.entry(name.name.clone()).or_insert(entity);
				}
				if let Ok(tag) = world.get::<&Tag>(entity) {
					components.insert((entity, ComponentKind::Tag));
					tags.entry(tag.tag.clone()).or_default().push(entity);
				}
				if world.get::<&Transform>(entity).is_ok() {
					components.insert((entity, ComponentKind::Transform));
					// World-space, not the raw local `Transform`, so it matches what raycasts
					// (which operate in the physics world's coordinate space) expect.
					if let Some(world_transform) = scene.world_transform(entity) {
						transform_positions.insert(entity, (world_transform.position.x, world_transform.position.y));
					}
				}
				if world.get::<&Parent>(entity).is_ok() {
					components.insert((entity, ComponentKind::Parent));
				}
				if world.get::<&Children>(entity).is_ok() {
					components.insert((entity, ComponentKind::Children));
				}
				if world.get::<&Inactive>(entity).is_ok() {
					components.insert((entity, ComponentKind::Inactive));
				}
				if world.get::<&RigidBody>(entity).is_ok() {
					components.insert((entity, ComponentKind::Rigidbody));
				}
				if world.get::<&PhysicsSettings>(entity).is_ok() {
					components.insert((entity, ComponentKind::PhysicsSettings));
				}
				if world.get::<&Collider>(entity).is_ok() {
					components.insert((entity, ComponentKind::Collider));
				}
				if let Ok(sprite) = world.get::<&Sprite>(entity) {
					components.insert((entity, ComponentKind::Sprite));
					sprite_texture_rotations.insert(entity, sprite.settings.texture_rotation_degrees);
				}
				if world.get::<&Camera>(entity).is_ok() {
					components.insert((entity, ComponentKind::Camera));
				}
				if world.get::<&Scripts>(entity).is_ok_and(|s| !s.list.is_empty()) {
					components.insert((entity, ComponentKind::Script));
				}
				if let Ok(button) = world.get::<&UIButton>(entity) {
					components.insert((entity, ComponentKind::UIButton));
					// `pressed` is written by UISystem from yakui's edge-triggered
					// click event (true for exactly the frame the click completes),
					// so caching it here already gives IsClicked() correct
					// once-per-click semantics rather than held-state polling.
					button_clicked.insert(entity, button.pressed);
				}
				if world.get::<&UIBar>(entity).is_ok() {
					components.insert((entity, ComponentKind::UIBar));
				}
				if world.get::<&UIText>(entity).is_ok() {
					components.insert((entity, ComponentKind::UIText));
				}
				if world.get::<&UIImage>(entity).is_ok() {
					components.insert((entity, ComponentKind::UIImage));
				}
				if world.get::<&AudioSource>(entity).is_ok() {
					components.insert((entity, ComponentKind::Audio));
				}
				if world.get::<&Animator>(entity).is_ok() {
					components.insert((entity, ComponentKind::Animator));
				}
			}
		});

		API_STATE.with(|state| {
			let mut state = state.borrow_mut();
			state.components = components;
			state.transform_positions = transform_positions;
			state.sprite_texture_rotations = sprite_texture_rotations;
			state.button_clicked = button_clicked;
			state.names = names;
			state.tags = tags;
			state.entities_by_index = entities_by_index;
		});
	}

	pub fn refresh_velocity_cache(velocities: impl IntoIterator<Item = (EntityId, f32, f32)>) {
		API_STATE.with(|state| {
			state.borrow_mut().velocities = velocities.into_iter().map(|(entity, x, y)| (entity, (x, y))).collect();
		});
	}

	pub fn refresh_audio_playing_cache(playing: impl IntoIterator<Item = (EntityId, bool)>) {
		API_STATE.with(|state| {
			state.borrow_mut().audio_playing = playing.into_iter().collect();
		});
	}

	pub extern "C" fn is_key_pressed(key: i32) -> bool { Input::is_key_pressed(key_code_from_i32(key)) }

	pub extern "C" fn is_key_just_pressed(key: i32) -> bool { Input::is_key_just_pressed(key_code_from_i32(key)) }

	pub extern "C" fn is_key_released(key: i32) -> bool { Input::is_key_released(key_code_from_i32(key)) }

	pub extern "C" fn is_key_just_released(key: i32) -> bool { Input::is_key_just_released(key_code_from_i32(key)) }

	pub extern "C" fn is_mouse_button_pressed(button: i32) -> bool {
		Input::is_mouse_button_pressed(mouse_code_from_i32(button))
	}

	pub extern "C" fn is_mouse_button_just_pressed(button: i32) -> bool {
		Input::is_mouse_button_just_pressed(mouse_code_from_i32(button))
	}

	pub extern "C" fn is_mouse_button_released(button: i32) -> bool {
		Input::is_mouse_button_released(mouse_code_from_i32(button))
	}

	pub extern "C" fn is_mouse_button_just_released(button: i32) -> bool {
		Input::is_mouse_button_just_released(mouse_code_from_i32(button))
	}

	pub extern "C" fn get_mouse_position(out_x: *mut f32, out_y: *mut f32) {
		if out_x.is_null() || out_y.is_null() {
			return;
		}

		let position = Input::get_mouse_pos();
		write_position(out_x, out_y, position.x, position.y);
	}

	pub extern "C" fn get_mouse_delta(out_x: *mut f32, out_y: *mut f32) {
		if out_x.is_null() || out_y.is_null() {
			return;
		}

		let delta = Input::get_mouse_delta();
		write_position(out_x, out_y, delta.x, delta.y);
	}

	pub extern "C" fn get_time_state_ptr() -> *const ScriptingTimeState { TIME_STATE.0.get().cast_const() }

	pub extern "C" fn has_component(entity: u64, component_type: u32) -> bool {
		let entity = script_arg_to_entity_id(entity);
		let Some(component_kind) = component_kind_from_u32(component_type) else {
			return false;
		};

		API_STATE.with(|state| state.borrow().components.contains(&(entity, component_kind)))
	}

	pub extern "C" fn get_position(entity: u64, out_x: *mut f32, out_y: *mut f32) {
		if out_x.is_null() || out_y.is_null() {
			return;
		}

		let entity = script_arg_to_entity_id(entity);
		let (x, y) = API_STATE
			.with(|state| state.borrow().transform_positions.get(&entity).copied())
			.unwrap_or((0.0, 0.0));

		write_position(out_x, out_y, x, y);
	}

	pub extern "C" fn get_velocity(entity: u64, out_x: *mut f32, out_y: *mut f32) {
		if out_x.is_null() || out_y.is_null() {
			return;
		}

		let entity = script_arg_to_entity_id(entity);
		let (x, y) = API_STATE
			.with(|state| state.borrow().velocities.get(&entity).copied())
			.unwrap_or((0.0, 0.0));

		write_position(out_x, out_y, x, y);
	}

	pub extern "C" fn add_2d_force(entity: u64, x: f32, y: f32) {
		push_command(ScriptingApiCommand::Add2DForce {
			entity: script_arg_to_entity_id(entity),
			x,
			y,
		});
	}

	pub extern "C" fn add_2d_impulse(entity: u64, x: f32, y: f32) {
		push_command(ScriptingApiCommand::Add2DImpulse {
			entity: script_arg_to_entity_id(entity),
			x,
			y,
		});
	}

	pub extern "C" fn set_velocity(entity: u64, x: f32, y: f32) {
		push_command(ScriptingApiCommand::SetVelocity {
			entity: script_arg_to_entity_id(entity),
			x,
			y,
		});
	}

	pub extern "C" fn play_audio(entity: u64) {
		push_audio_command(AudioApiCommand::Play {
			entity: script_arg_to_entity_id(entity),
		});
	}

	pub extern "C" fn stop_audio(entity: u64) {
		push_audio_command(AudioApiCommand::Stop {
			entity: script_arg_to_entity_id(entity),
		});
	}

	pub extern "C" fn set_audio_volume(entity: u64, volume: f32) {
		push_audio_command(AudioApiCommand::SetVolume {
			entity: script_arg_to_entity_id(entity),
			volume,
		});
	}

	pub extern "C" fn is_audio_playing(entity: u64) -> bool {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| state.borrow().audio_playing.get(&entity).copied().unwrap_or(false))
	}

	#[allow(clippy::too_many_arguments)]
	pub extern "C" fn raycast_2d(
		origin_x: f32,
		origin_y: f32,
		dir_x: f32,
		dir_y: f32,
		max_distance: f32,
		out_point_x: *mut f32,
		out_point_y: *mut f32,
		out_normal_x: *mut f32,
		out_normal_y: *mut f32,
		out_entity_id: *mut u64,
	) -> bool {
		let Some((context, query)) = RAYCAST_PROVIDER.with(Cell::get) else {
			return false;
		};

		let Some(hit) = query(
			context,
			Vec2::new(origin_x, origin_y),
			Vec2::new(dir_x, dir_y),
			max_distance,
		) else {
			return false;
		};

		if !out_point_x.is_null() && !out_point_y.is_null() {
			write_position(out_point_x, out_point_y, hit.point.x, hit.point.y);
		}
		if !out_normal_x.is_null() && !out_normal_y.is_null() {
			write_position(out_normal_x, out_normal_y, hit.normal.x, hit.normal.y);
		}
		if !out_entity_id.is_null() {
			unsafe {
				*out_entity_id = entity_id_to_script_arg(hit.entity);
			}
		}

		true
	}

	pub extern "C" fn get_sprite_texture_rotation_degrees(entity: u64) -> f32 {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow()
				.sprite_texture_rotations
				.get(&entity)
				.copied()
				.unwrap_or(0.0)
		})
	}

	pub extern "C" fn set_sprite_texture_rotation_degrees(entity: u64, degrees: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			let mut state = state.borrow_mut();
			state.sprite_texture_rotations.insert(entity, degrees);
			state
				.scene_commands
				.push(ScriptingSceneCommand::SetSpriteTextureRotationDegrees { entity, degrees });
		});
	}

	pub extern "C" fn set_button_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetButtonColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_button_hover_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetButtonHoverColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_bar_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetBarColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_bar_bg_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetBarBgColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_text_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetTextColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_bar_label(entity: u64, text: *const u8, len: i32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(text) = read_utf8(text, len) else {
			ze_log::error!(target: "zeroengine.script", "script Bar.SetLabel bridge received an invalid text buffer");
			return;
		};
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetBarLabel { entity, text });
		});
	}

	pub extern "C" fn set_button_pressed_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetButtonPressedColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_button_text(entity: u64, text: *const u8, len: i32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(text) = read_utf8(text, len) else {
			ze_log::error!(target: "zeroengine.script", "script Button.SetText bridge received an invalid text buffer");
			return;
		};
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetButtonText { entity, text });
		});
	}

	pub extern "C" fn set_text_text(entity: u64, text: *const u8, len: i32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(text) = read_utf8(text, len) else {
			ze_log::error!(target: "zeroengine.script", "script Text.SetText bridge received an invalid text buffer");
			return;
		};
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetTextText { entity, text });
		});
	}

	pub extern "C" fn set_text_font_size(entity: u64, font_size: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetTextFontSize { entity, font_size });
		});
	}

	pub extern "C" fn is_button_clicked(entity: u64) -> bool {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| state.borrow().button_clicked.get(&entity).copied().unwrap_or(false))
	}

	pub extern "C" fn set_image_color(entity: u64, r: f32, g: f32, b: f32, a: f32) {
		let entity = script_arg_to_entity_id(entity);
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetImageColor { entity, r, g, b, a });
		});
	}

	pub extern "C" fn set_image_texture_path(entity: u64, path: *const u8, len: i32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(path) = read_utf8(path, len) else {
			ze_log::error!(target: "zeroengine.script", "script Image.SetTexturePath bridge received an invalid path buffer");
			return;
		};
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetImageTexturePath { entity, path });
		});
	}

	pub extern "C" fn set_image_sheet_cell(entity: u64, sheet_path: *const u8, sheet_path_len: i32, cell_id: u32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(sheet_path) = read_utf8(sheet_path, sheet_path_len) else {
			ze_log::error!(target: "zeroengine.script", "script Image.SetSheetCell bridge received an invalid sheet path buffer");
			return;
		};
		API_STATE.with(|state| {
			state
				.borrow_mut()
				.scene_commands
				.push(ScriptingSceneCommand::SetImageSheetCell {
					entity,
					sheet_path,
					cell_id,
				});
		});
	}

	pub extern "C" fn scene_load(name: *const u8, len: i32) {
		match read_utf8(name, len) {
			Some(name) => {
				let name = name.trim();
				if name.is_empty() {
					ze_log::error!(target: "zeroengine.script", "script Scene.Load bridge received an empty scene name");
					return;
				}
				push_scene_load_command(ScriptingSceneLoadCommand::Load { name: name.to_string() });
			}
			None => {
				ze_log::error!(target: "zeroengine.script", "script Scene.Load bridge received an invalid scene name buffer");
			}
		}
	}

	pub extern "C" fn scene_load_main() { push_scene_load_command(ScriptingSceneLoadCommand::LoadMain); }

	pub extern "C" fn scene_reload() { push_scene_load_command(ScriptingSceneLoadCommand::Reload); }

	pub extern "C" fn quit_game() { super::SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst); }

	pub extern "C" fn set_animator_state(entity: u64, text: *const u8, len: i32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(state) = read_utf8(text, len) else {
			ze_log::error!(target: "zeroengine.script", "script Animator.SetState bridge received an invalid state name buffer");
			return;
		};
		push_animator_command(AnimatorApiCommand::SetState { entity, state });
	}

	pub extern "C" fn find_entity_by_name(name: *const u8, len: i32) -> u64 {
		let Some(name) = read_utf8(name, len) else {
			ze_log::error!(target: "zeroengine.script", "script Entity.Find bridge received an invalid name buffer");
			return INVALID_ENTITY;
		};
		API_STATE.with(|state| {
			state
				.borrow()
				.names
				.get(&name)
				.map_or(INVALID_ENTITY, |entity| entity_id_to_script_arg(*entity))
		})
	}

	pub extern "C" fn find_entity_by_tag(tag: *const u8, len: i32) -> u64 {
		let Some(tag) = read_utf8(tag, len) else {
			ze_log::error!(target: "zeroengine.script", "script Entity.FindWithTag bridge received an invalid tag buffer");
			return INVALID_ENTITY;
		};
		API_STATE.with(|state| {
			state
				.borrow()
				.tags
				.get(&tag)
				.and_then(|entities| entities.first())
				.map_or(INVALID_ENTITY, |entity| entity_id_to_script_arg(*entity))
		})
	}

	/// Writes up to `capacity` matching entity ids into `out`, returning the
	/// total number of matches (which may exceed `capacity` -- the C# side
	/// calls once with a null/zero buffer to size, then again to fill).
	pub extern "C" fn find_entities_by_tag(tag: *const u8, len: i32, out: *mut u64, capacity: i32) -> i32 {
		let Some(tag) = read_utf8(tag, len) else {
			ze_log::error!(target: "zeroengine.script", "script Entity.FindAllWithTag bridge received an invalid tag buffer");
			return -1;
		};

		API_STATE.with(|state| {
			let state = state.borrow();
			let Some(entities) = state.tags.get(&tag) else {
				return 0;
			};

			if !out.is_null() && capacity > 0 {
				let writable = usize::min(entities.len(), capacity as usize);
				for (index, entity) in entities.iter().take(writable).enumerate() {
					unsafe {
						*out.add(index) = entity_id_to_script_arg(*entity);
					}
				}
			}

			i32::try_from(entities.len()).unwrap_or(i32::MAX)
		})
	}

	pub extern "C" fn find_entity_by_id(index: u32) -> u64 {
		API_STATE.with(|state| {
			state
				.borrow()
				.entities_by_index
				.get(&index)
				.map_or(INVALID_ENTITY, |entity| entity_id_to_script_arg(*entity))
		})
	}

	pub extern "C" fn instantiate_entity(
		template: u64,
		pos_x: f32,
		pos_y: f32,
		rotation_degrees: f32,
		out_entity: *mut u64,
	) -> bool {
		let template = script_arg_to_entity_id(template);
		let new_entity = with_scene(|scene| {
			let new_entity = scene.clone_entity(template)?;
			if let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(new_entity) {
				transform.position.x = pos_x;
				transform.position.y = pos_y;
				transform.rotation = ze_core::Quat::from_rotation_z(rotation_degrees.to_radians());
			}
			Some(new_entity)
		})
		.flatten();

		if !out_entity.is_null() {
			unsafe {
				*out_entity = new_entity.map_or(INVALID_ENTITY, entity_id_to_script_arg);
			}
		}

		new_entity.is_some()
	}

	pub extern "C" fn destroy_entity(entity: u64) {
		let entity = script_arg_to_entity_id(entity);
		with_scene(|scene| scene.destroy_entity(entity));
	}

	pub extern "C" fn add_component(entity: u64, component_type: u32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(kind) = component_kind_from_u32(component_type) else {
			return;
		};
		with_scene(|scene| add_default_component(scene, entity, kind));
	}

	pub extern "C" fn remove_component(entity: u64, component_type: u32) {
		let entity = script_arg_to_entity_id(entity);
		let Some(kind) = component_kind_from_u32(component_type) else {
			return;
		};
		with_scene(|scene| remove_component_of_kind(scene, entity, kind));
	}

	pub extern "C" fn get_mouse_world_position(out_x: *mut f32, out_y: *mut f32) {
		if out_x.is_null() || out_y.is_null() {
			return;
		}

		let screen = Input::get_mouse_pos();
		let world = with_scene(|scene| {
			scene
				.world()
				.borrow::<UniqueView<ActiveCameraView>>()
				.ok()
				.and_then(|camera| camera.unproject_to_world(screen))
		})
		.flatten();

		// No active camera view yet -> fall back to raw screen coordinates rather
		// than lying with (0, 0).
		let position = world.unwrap_or(screen);
		write_position(out_x, out_y, position.x, position.y);
	}

	pub extern "C" fn set_cursor_visible(visible: i32) {
		push_cursor_command(CursorApiCommand::SetVisible(visible != 0));
	}

	pub extern "C" fn set_cursor_grab_mode(mode: i32) {
		let mode = match mode {
			1 => ScriptingCursorGrabMode::Confined,
			2 => ScriptingCursorGrabMode::Locked,
			_ => ScriptingCursorGrabMode::None,
		};
		push_cursor_command(CursorApiCommand::SetGrabMode(mode));
	}

	/// Adds a default-constructed component of `kind` to `entity`. Only the
	/// component kinds that have a meaningful zero-argument default are
	/// supported; the rest (sprites, cameras, UI widgets, hierarchy links)
	/// carry data that a script can't sensibly conjure, so they are logged and
	/// skipped.
	fn add_default_component(scene: &mut Scene, entity: EntityId, kind: ComponentKind) {
		let world = scene.world_mut();
		match kind {
			ComponentKind::Transform => world.add_component(entity, (Transform::default(),)),
			ComponentKind::Rigidbody => world.add_component(entity, (RigidBody::default(),)),
			ComponentKind::Collider => world.add_component(entity, (Collider::default(),)),
			ComponentKind::PhysicsSettings => world.add_component(entity, (PhysicsSettings::default(),)),
			ComponentKind::Animator => world.add_component(entity, (Animator::default(),)),
			ComponentKind::Audio => world.add_component(entity, (AudioSource::default(),)),
			ComponentKind::Inactive => world.add_component(entity, (Inactive,)),
			ComponentKind::Tag => world.add_component(entity, (Tag { tag: String::new() },)),
			ComponentKind::Name => world.add_component(entity, (Name { name: String::new() },)),
			other => {
				ze_log::warn!(
					target: "zeroengine.script",
					"AddComponent is not supported for component kind {other:?}; skipping"
				);
			}
		}
	}

	fn remove_component_of_kind(scene: &mut Scene, entity: EntityId, kind: ComponentKind) {
		let world = scene.world_mut();
		match kind {
			ComponentKind::Name => drop(world.remove::<(Name,)>(entity)),
			ComponentKind::Tag => drop(world.remove::<(Tag,)>(entity)),
			ComponentKind::Transform => drop(world.remove::<(Transform,)>(entity)),
			ComponentKind::Parent => drop(world.remove::<(Parent,)>(entity)),
			ComponentKind::Children => drop(world.remove::<(Children,)>(entity)),
			ComponentKind::Inactive => drop(world.remove::<(Inactive,)>(entity)),
			ComponentKind::Rigidbody => drop(world.remove::<(RigidBody,)>(entity)),
			ComponentKind::PhysicsSettings => drop(world.remove::<(PhysicsSettings,)>(entity)),
			ComponentKind::Collider => drop(world.remove::<(Collider,)>(entity)),
			ComponentKind::Sprite => drop(world.remove::<(Sprite,)>(entity)),
			ComponentKind::Camera => drop(world.remove::<(Camera,)>(entity)),
			ComponentKind::UIButton => drop(world.remove::<(UIButton,)>(entity)),
			ComponentKind::UIBar => drop(world.remove::<(UIBar,)>(entity)),
			ComponentKind::UIText => drop(world.remove::<(UIText,)>(entity)),
			ComponentKind::UIImage => drop(world.remove::<(UIImage,)>(entity)),
			ComponentKind::Audio => drop(world.remove::<(AudioSource,)>(entity)),
			ComponentKind::Animator => drop(world.remove::<(Animator,)>(entity)),
			ComponentKind::Script => {
				ze_log::warn!(
					target: "zeroengine.script",
					"RemoveComponent is not supported for script components; skipping"
				);
			}
		}
	}

	pub extern "C" fn log_info(message: *const u8, len: i32) {
		match read_utf8(message, len) {
			Some(message) => {
				ze_log::info!(target: "zeroengine.script", "{message}");
			}
			None => {
				ze_log::error!(target: "zeroengine.script", "script Log.Info bridge received an invalid message buffer");
			}
		}
	}

	pub extern "C" fn log_warn(message: *const u8, len: i32) {
		match read_utf8(message, len) {
			Some(message) => {
				ze_log::warn!(target: "zeroengine.script", "{message}");
			}
			None => {
				ze_log::error!(target: "zeroengine.script", "script Log.Warn bridge received an invalid message buffer");
			}
		}
	}

	pub extern "C" fn log_error(message: *const u8, len: i32) {
		match read_utf8(message, len) {
			Some(message) => {
				ze_log::error!(target: "zeroengine.script", "{message}");
			}
			None => {
				ze_log::error!(target: "zeroengine.script", "script Log.Error bridge received an invalid message buffer");
			}
		}
	}

	pub extern "C" fn log_debug(message: *const u8, len: i32) {
		match read_utf8(message, len) {
			Some(message) => {
				ze_log::debug!(target: "zeroengine.script", "{message}");
			}
			None => {
				ze_log::error!(target: "zeroengine.script", "script Log.Debug bridge received an invalid message buffer");
			}
		}
	}

	fn push_command(command: ScriptingApiCommand) {
		API_STATE.with(|state| {
			state.borrow_mut().commands.push(command);
		});
	}

	fn push_audio_command(command: AudioApiCommand) {
		API_STATE.with(|state| {
			state.borrow_mut().audio_commands.push(command);
		});
	}

	fn push_animator_command(command: AnimatorApiCommand) {
		API_STATE.with(|state| {
			state.borrow_mut().animator_commands.push(command);
		});
	}

	fn push_scene_load_command(command: ScriptingSceneLoadCommand) {
		API_STATE.with(|state| {
			state.borrow_mut().scene_load_commands.push(command);
		});
	}

	fn push_cursor_command(command: CursorApiCommand) {
		API_STATE.with(|state| {
			state.borrow_mut().cursor_commands.push(command);
		});
	}

	fn write_position(out_x: *mut f32, out_y: *mut f32, x: f32, y: f32) {
		unsafe {
			*out_x = x;
			*out_y = y;
		}
	}

	fn read_utf8(message: *const u8, len: i32) -> Option<String> {
		if len < 0 {
			return None;
		}

		if len == 0 {
			return Some(String::new());
		}

		if message.is_null() {
			return None;
		}

		let bytes = unsafe { std::slice::from_raw_parts(message, len as usize) };
		Some(String::from_utf8_lossy(bytes).into_owned())
	}

	const fn key_code_from_i32(key: i32) -> ZKeyCode {
		match key {
			0 => ZKeyCode::Escape,
			1 => ZKeyCode::Space,
			2 => ZKeyCode::Q,
			3 => ZKeyCode::W,
			4 => ZKeyCode::E,
			5 => ZKeyCode::R,
			6 => ZKeyCode::T,
			7 => ZKeyCode::Y,
			8 => ZKeyCode::U,
			9 => ZKeyCode::I,
			10 => ZKeyCode::O,
			11 => ZKeyCode::P,
			12 => ZKeyCode::A,
			13 => ZKeyCode::S,
			14 => ZKeyCode::D,
			15 => ZKeyCode::F,
			16 => ZKeyCode::G,
			17 => ZKeyCode::H,
			18 => ZKeyCode::J,
			19 => ZKeyCode::K,
			20 => ZKeyCode::L,
			21 => ZKeyCode::Z,
			22 => ZKeyCode::X,
			23 => ZKeyCode::C,
			24 => ZKeyCode::V,
			25 => ZKeyCode::B,
			26 => ZKeyCode::N,
			27 => ZKeyCode::M,
			28 => ZKeyCode::Enter,
			29 => ZKeyCode::LCtrl,
			30 => ZKeyCode::LShift,
			31 => ZKeyCode::RCtrl,
			32 => ZKeyCode::RShift,
			33 => ZKeyCode::ArrowUp,
			34 => ZKeyCode::ArrowDown,
			35 => ZKeyCode::ArrowLeft,
			36 => ZKeyCode::ArrowRight,
			37 => ZKeyCode::K1,
			38 => ZKeyCode::K2,
			39 => ZKeyCode::K3,
			40 => ZKeyCode::K4,
			41 => ZKeyCode::K5,
			42 => ZKeyCode::K6,
			43 => ZKeyCode::K7,
			44 => ZKeyCode::K8,
			45 => ZKeyCode::K9,
			46 => ZKeyCode::K0,
			47 => ZKeyCode::KF1,
			48 => ZKeyCode::KF2,
			49 => ZKeyCode::KF3,
			50 => ZKeyCode::KF4,
			51 => ZKeyCode::KF5,
			52 => ZKeyCode::KF6,
			53 => ZKeyCode::KF7,
			54 => ZKeyCode::KF8,
			55 => ZKeyCode::KF9,
			56 => ZKeyCode::KF10,
			57 => ZKeyCode::KF11,
			58 => ZKeyCode::KF12,
			59 => ZKeyCode::LAlt,
			60 => ZKeyCode::RAlt,
			61 => ZKeyCode::LSuper,
			62 => ZKeyCode::RSuper,
			63 => ZKeyCode::CapsLock,
			64 => ZKeyCode::Tab,
			65 => ZKeyCode::Backspace,
			66 => ZKeyCode::ContextMenu,
			_ => ZKeyCode::Unknown,
		}
	}

	const fn mouse_code_from_i32(button: i32) -> ZMouseCode {
		match button {
			0 => ZMouseCode::Left,
			1 => ZMouseCode::Right,
			2 => ZMouseCode::Middle,
			3 => ZMouseCode::Back,
			4 => ZMouseCode::Forward,
			_ => ZMouseCode::Other,
		}
	}

	const fn component_kind_from_u32(component_type: u32) -> Option<ComponentKind> {
		match component_type {
			1 => Some(ComponentKind::Name),
			2 => Some(ComponentKind::Tag),
			3 => Some(ComponentKind::Transform),
			4 => Some(ComponentKind::Parent),
			5 => Some(ComponentKind::Children),
			6 => Some(ComponentKind::Inactive),
			7 => Some(ComponentKind::Rigidbody),
			8 => Some(ComponentKind::PhysicsSettings),
			9 => Some(ComponentKind::Collider),
			10 => Some(ComponentKind::Sprite),
			11 => Some(ComponentKind::Camera),
			12 => Some(ComponentKind::Script),
			13 => Some(ComponentKind::UIButton),
			14 => Some(ComponentKind::UIBar),
			15 => Some(ComponentKind::UIText),
			16 => Some(ComponentKind::Audio),
			17 => Some(ComponentKind::Animator),
			18 => Some(ComponentKind::UIImage),
			_ => None,
		}
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn script_key_codes_include_modifier_keys() {
			assert_eq!(key_code_from_i32(29), ZKeyCode::LCtrl);
			assert_eq!(key_code_from_i32(30), ZKeyCode::LShift);
			assert_eq!(key_code_from_i32(31), ZKeyCode::RCtrl);
			assert_eq!(key_code_from_i32(32), ZKeyCode::RShift);
			assert_eq!(key_code_from_i32(59), ZKeyCode::LAlt);
			assert_eq!(key_code_from_i32(60), ZKeyCode::RAlt);
			assert_eq!(key_code_from_i32(61), ZKeyCode::LSuper);
			assert_eq!(key_code_from_i32(62), ZKeyCode::RSuper);
			assert_eq!(key_code_from_i32(66), ZKeyCode::ContextMenu);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn script_entity_id_round_trips() {
		let entity = EntityId::new_from_index_and_gen(4, 0);
		let script_arg = entity_id_to_script_arg(entity);

		assert_eq!(script_arg_to_entity_id(script_arg), entity);
	}

	#[test]
	fn set_velocity_queues_a_command() {
		let _ = api::drain_commands();
		let entity = EntityId::new_from_index_and_gen(2, 0);

		api::set_velocity(entity_id_to_script_arg(entity), 1.5, -2.0);

		let commands = api::drain_commands();
		assert_eq!(commands.len(), 1);
		match commands[0] {
			ScriptingApiCommand::SetVelocity { entity: e, x, y } => {
				assert_eq!(e, entity);
				assert!((x - 1.5).abs() < f32::EPSILON);
				assert!((y + 2.0).abs() < f32::EPSILON);
			}
			other => panic!("expected SetVelocity, got {other:?}"),
		}
	}

	#[test]
	fn cursor_calls_queue_visibility_and_grab_commands() {
		let _ = api::drain_cursor_commands();

		api::set_cursor_visible(0);
		api::set_cursor_grab_mode(2);

		let commands = api::drain_cursor_commands();
		assert_eq!(commands.len(), 2);
		assert!(matches!(commands[0], CursorApiCommand::SetVisible(false)));
		assert!(matches!(
			commands[1],
			CursorApiCommand::SetGrabMode(ScriptingCursorGrabMode::Locked)
		));
	}

	#[test]
	fn find_entity_bridges_read_the_scene_cache() {
		let mut scene = Scene::new("Find Test");
		let hero = scene.create_entity("Hero");
		scene.world_mut().add_component(
			hero,
			(ze_ecs::Tag {
				tag: "enemy".to_string(),
			},),
		);
		let goblin = scene.create_entity("Goblin");
		scene.world_mut().add_component(
			goblin,
			(ze_ecs::Tag {
				tag: "enemy".to_string(),
			},),
		);

		api::refresh_scene_cache(&scene);

		let hero_name = b"Hero";
		assert_eq!(
			api::find_entity_by_name(hero_name.as_ptr(), i32::try_from(hero_name.len()).expect("length fits in i32")),
			entity_id_to_script_arg(hero)
		);

		let missing = b"Nobody";
		assert_eq!(
			api::find_entity_by_name(missing.as_ptr(), i32::try_from(missing.len()).expect("length fits in i32")),
			u64::MAX
		);

		let tag = b"enemy";
		assert_eq!(
			api::find_entities_by_tag(tag.as_ptr(), i32::try_from(tag.len()).expect("length fits in i32"), std::ptr::null_mut(), 0),
			2
		);

		let hero_index = u32::try_from(hero.index() & 0xffff_ffff).expect("index fits in u32");
		assert_eq!(api::find_entity_by_id(hero_index), entity_id_to_script_arg(hero));
		assert_eq!(api::find_entity_by_id(9_999), u64::MAX);
	}

	#[test]
	fn script_field_metadata_accepts_csharp_property_names() -> Result<()> {
		let fields: Vec<ScriptFieldMetadata> = serde_json::from_str(
			r#"[
				{"name":"speed","type":"float","source":"public","defaultValue":{"kind":"float","value":0.5}},
				{"Name":"health","Type":"int","Source":"zeroField","DefaultValue":{"kind":"int","value":100}},
				{"name":"legacy","type":"bool"}
			]"#,
		)?;

		assert_eq!(fields[0].name, "speed");
		assert_eq!(fields[0].ty, "float");
		assert_eq!(fields[0].source, ScriptFieldSource::Public);
		let Some(ScriptFieldValue::Float(speed)) = fields[0].default_value.as_ref() else {
			return Err(anyhow!("expected speed default value"));
		};
		assert!((*speed - 0.5).abs() < f32::EPSILON);
		assert_eq!(fields[1].name, "health");
		assert_eq!(fields[1].ty, "int");
		assert_eq!(fields[1].source, ScriptFieldSource::ZeroField);
		let Some(ScriptFieldValue::Int(health)) = fields[1].default_value.as_ref() else {
			return Err(anyhow!("expected health default value"));
		};
		assert_eq!(*health, 100);
		assert_eq!(fields[2].source, ScriptFieldSource::Public);
		assert!(fields[2].default_value.is_none());
		Ok(())
	}

	#[test]
	fn script_field_sync_removes_values_missing_from_metadata() -> Result<()> {
		let mut scene = Scene::new("Script Field Sync Test");
		register_scripting_components(scene.registry_mut());
		let entity = scene.create_entity("Scripted");
		scene.entity_mut(entity).add_component(Scripts {
			list: vec![Script {
				path: "Game.Player".to_string(),
				enabled: true,
				fields: BTreeMap::from([
					("speed".to_string(), ScriptFieldValue::Float(1.5)),
					("removed".to_string(), ScriptFieldValue::Int(7)),
				]),
			}],
		});

		let pruned = sync_scene_script_fields_with(&mut scene, |class_path| {
			assert_eq!(class_path, "Game.Player");
			Ok(Some(vec![ScriptFieldMetadata {
				name: "speed".to_string(),
				ty: "float".to_string(),
				source: ScriptFieldSource::ZeroField,
				default_value: None,
			}]))
		})?;

		let scripts = scene.world().get::<&Scripts>(entity)?;
		let script = scripts
			.list
			.iter()
			.find(|s| s.path == "Game.Player")
			.expect("test script should exist");
		assert_eq!(pruned, 1);
		assert!(script.fields.contains_key("speed"));
		assert!(!script.fields.contains_key("removed"));
		Ok(())
	}
}
