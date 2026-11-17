//! The `RenderPass` trait, `PassDesc` with its slots and parameters, and the
//! `PassRegistry`.
//!
//! Names are `Cow<'static, str>`: the passes of the engine give `&'static`
//! ones, a pass written in a script (plan, section 20) gives its own.

use std::{borrow::Cow, collections::BTreeMap, sync::Arc};

use serde_json::Value;
use zenith_error::ZResult;

use super::exec::{ExecContext, InitContext, PrepareContext};

/// A name in a description: a slot, a parameter, a type of pass.
pub type Name = Cow<'static, str>;

/// What a slot carries.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SlotKind {
	/// A color texture.
	Color,
	/// A depth texture.
	Depth,
	/// Data that prepare made for the view (see [`data`]), by the name of its
	/// kind. The graph only hands it on.
	Data(Name),
}

impl SlotKind {
	pub const fn data(kind: &'static str) -> Self {
		Self::Data(Cow::Borrowed(kind))
	}

	pub const fn is_texture(&self) -> bool {
		matches!(self, Self::Color | Self::Depth)
	}
}

/// The kinds of data the input nodes of the engine give.
pub mod data {
	pub const CAMERA: &str = "camera";
	pub const LIGHTS: &str = "lights";
	pub const ENVIRONMENT: &str = "environment";
	pub const MESHES: &str = "meshes";
	pub const SPRITES: &str = "sprites";
}

/// What a pass does with an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
	Read,
	/// It draws into the texture: a new version of it, which the readers of
	/// the old one must come before.
	Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDesc {
	pub name: Name,
	pub kind: SlotKind,
	pub access: Access,
	/// Not linked: the pass gets nothing there and has to do without (a
	/// neutral default). A required input that is not linked drops the pass.
	pub optional: bool,
}

impl InputDesc {
	pub fn read(name: impl Into<Name>, kind: SlotKind) -> Self {
		Self {
			name: name.into(),
			kind,
			access: Access::Read,
			optional: false,
		}
	}

	pub fn write(name: impl Into<Name>, kind: SlotKind) -> Self {
		Self {
			access: Access::Write,
			..Self::read(name, kind)
		}
	}

	#[must_use]
	pub fn optional(self) -> Self {
		Self { optional: true, ..self }
	}
}

/// The format of a texture a pass creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FormatRule {
	Fixed(wgpu::TextureFormat),
	/// Light brighter than white: `Rgba16Float`.
	Hdr,
	/// `Depth32Float`.
	Depth,
	/// The target of the camera itself (a window, a texture), not a texture of
	/// the graph. What the output node takes.
	ViewTarget,
}

impl FormatRule {
	/// The format, given that of the target of the view.
	pub const fn resolve(self, target: wgpu::TextureFormat) -> wgpu::TextureFormat {
		match self {
			Self::Fixed(format) => format,
			Self::Hdr => wgpu::TextureFormat::Rgba16Float,
			Self::Depth => wgpu::TextureFormat::Depth32Float,
			Self::ViewTarget => target,
		}
	}
}

/// The size of a texture a pass creates.
#[derive(Clone, Debug, PartialEq)]
pub enum SizeRule {
	/// A share of the size of the view (1 is the same size).
	View {
		scale: f32,
	},
	Absolute(u32, u32),
	/// From a parameter of the pass: an `Int` (a square) or a `Vec2`.
	FromParam(Name),
}

/// A texture a pass creates.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureDesc {
	pub format: FormatRule,
	pub size: SizeRule,
}

impl TextureDesc {
	/// The target of the camera.
	pub const VIEW_TARGET: Self = Self {
		format: FormatRule::ViewTarget,
		size: SizeRule::View { scale: 1.0 },
	};
	/// HDR color the size of the view.
	pub const HDR: Self = Self {
		format: FormatRule::Hdr,
		size: SizeRule::View { scale: 1.0 },
	};
	/// Depth the size of the view.
	pub const DEPTH: Self = Self {
		format: FormatRule::Depth,
		size: SizeRule::View { scale: 1.0 },
	};
}

/// Where an output comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputSource {
	/// A texture the pass creates (and the graph clears for it).
	Create(TextureDesc),
	/// The texture of an input, as the pass left it.
	PassThrough(Name),
	/// Data the pass publishes in `prepare` ([`PrepareContext::set_data`]).
	Data,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputDesc {
	pub name: Name,
	pub kind: SlotKind,
	pub source: OutputSource,
}

impl OutputDesc {
	pub fn create(name: impl Into<Name>, kind: SlotKind, texture: TextureDesc) -> Self {
		Self {
			name: name.into(),
			kind,
			source: OutputSource::Create(texture),
		}
	}

	pub fn pass_through(name: impl Into<Name>, kind: SlotKind, input: impl Into<Name>) -> Self {
		Self {
			name: name.into(),
			kind,
			source: OutputSource::PassThrough(input.into()),
		}
	}

	pub fn data(name: impl Into<Name>, kind: &'static str) -> Self {
		Self {
			name: name.into(),
			kind: SlotKind::data(kind),
			source: OutputSource::Data,
		}
	}
}

/// What a pass is, for the color of its node and the menus of the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PassCategory {
	Input,
	Geometry,
	Lighting,
	Post,
	/// Ends the graph: what it reads is the target of the view. The passes it
	/// does not depend on are dropped.
	Output,
	Debug,
}

/// The type of a parameter: what the editor shows a widget for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamType {
	Bool,
	Int,
	Float,
	Vec2,
	Vec3,
	Vec4,
	/// Linear RGBA.
	Color,
	Enum(Vec<Name>),
	String,
}

/// The value of a parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
	Bool(bool),
	Int(i64),
	Float(f32),
	Vec2([f32; 2]),
	Vec3([f32; 3]),
	Vec4([f32; 4]),
	Color([f32; 4]),
	Enum(String),
	String(String),
}

impl ParamValue {
	/// The value of `json` as a parameter of `ty`; `None` if it is not one.
	pub fn from_json(ty: &ParamType, json: &Value) -> Option<Self> {
		let floats = |n: usize| -> Option<Vec<f32>> {
			let items = json.as_array()?;
			(items.len() == n)
				.then(|| items.iter().map(|item| item.as_f64().map(|v| v as f32)).collect())
				.flatten()
		};
		Some(match ty {
			ParamType::Bool => Self::Bool(json.as_bool()?),
			ParamType::Int => Self::Int(json.as_i64()?),
			ParamType::Float => Self::Float(json.as_f64()? as f32),
			ParamType::Vec2 => Self::Vec2(floats(2)?.try_into().ok()?),
			ParamType::Vec3 => Self::Vec3(floats(3)?.try_into().ok()?),
			ParamType::Vec4 => Self::Vec4(floats(4)?.try_into().ok()?),
			ParamType::Color => Self::Color(floats(4)?.try_into().ok()?),
			ParamType::Enum(variants) => {
				let value = json.as_str()?;
				variants
					.iter()
					.any(|v| v == value)
					.then(|| Self::Enum(value.to_owned()))?
			}
			ParamType::String => Self::String(json.as_str()?.to_owned()),
		})
	}

	/// The value as the file of a graph holds it.
	pub fn to_json(&self) -> Value {
		match self {
			Self::Bool(value) => Value::Bool(*value),
			Self::Int(value) => Value::from(*value),
			Self::Float(value) => Value::from(f64::from(*value)),
			Self::Vec2(values) => values.iter().map(|&v| f64::from(v)).collect(),
			Self::Vec3(values) => values.iter().map(|&v| f64::from(v)).collect(),
			Self::Vec4(values) | Self::Color(values) => values.iter().map(|&v| f64::from(v)).collect(),
			Self::Enum(value) | Self::String(value) => Value::String(value.clone()),
		}
	}
}

/// A parameter of a pass.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDesc {
	pub name: Name,
	pub ty: ParamType,
	pub default: ParamValue,
	/// For `Int` and `Float`: a value outside is clamped.
	pub min: Option<f64>,
	pub max: Option<f64>,
	pub tooltip: Name,
}

impl ParamDesc {
	fn new(name: impl Into<Name>, ty: ParamType, default: ParamValue) -> Self {
		Self {
			name: name.into(),
			ty,
			default,
			min: None,
			max: None,
			tooltip: Cow::Borrowed(""),
		}
	}

	pub fn bool(name: impl Into<Name>, default: bool) -> Self {
		Self::new(name, ParamType::Bool, ParamValue::Bool(default))
	}

	pub fn int(name: impl Into<Name>, default: i64) -> Self {
		Self::new(name, ParamType::Int, ParamValue::Int(default))
	}

	pub fn float(name: impl Into<Name>, default: f32) -> Self {
		Self::new(name, ParamType::Float, ParamValue::Float(default))
	}

	pub fn vec2(name: impl Into<Name>, default: [f32; 2]) -> Self {
		Self::new(name, ParamType::Vec2, ParamValue::Vec2(default))
	}

	pub fn vec3(name: impl Into<Name>, default: [f32; 3]) -> Self {
		Self::new(name, ParamType::Vec3, ParamValue::Vec3(default))
	}

	pub fn vec4(name: impl Into<Name>, default: [f32; 4]) -> Self {
		Self::new(name, ParamType::Vec4, ParamValue::Vec4(default))
	}

	pub fn color(name: impl Into<Name>, default: [f32; 4]) -> Self {
		Self::new(name, ParamType::Color, ParamValue::Color(default))
	}

	pub fn string(name: impl Into<Name>, default: impl Into<String>) -> Self {
		Self::new(name, ParamType::String, ParamValue::String(default.into()))
	}

	/// One of `variants`; `default` must be one of them.
	pub fn enumeration(name: impl Into<Name>, variants: &[&'static str], default: &str) -> Self {
		Self::new(
			name,
			ParamType::Enum(variants.iter().map(|&v| Cow::Borrowed(v)).collect()),
			ParamValue::Enum(default.to_owned()),
		)
	}

	#[must_use]
	#[allow(clippy::missing_const_for_fn)] // its destructor cannot run at compile time
	pub fn range(self, min: f64, max: f64) -> Self {
		Self {
			min: Some(min),
			max: Some(max),
			..self
		}
	}

	#[must_use]
	pub fn tooltip(self, tooltip: impl Into<Name>) -> Self {
		Self {
			tooltip: tooltip.into(),
			..self
		}
	}

	/// `value` clamped to the range.
	fn clamp(&self, value: ParamValue) -> ParamValue {
		let (min, max) = (self.min.unwrap_or(f64::MIN), self.max.unwrap_or(f64::MAX));
		match value {
			ParamValue::Int(v) => ParamValue::Int((v as f64).clamp(min, max) as i64),
			ParamValue::Float(v) => ParamValue::Float(f64::from(v).clamp(min, max) as f32),
			other => other,
		}
	}
}

/// The parameters of a node: every parameter of its pass, with a value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamMap {
	values: BTreeMap<String, ParamValue>,
}

impl ParamMap {
	/// The values of `json` for the parameters of `desc`; a parameter that is
	/// not there is its default. What is wrong (a value of another type, a
	/// parameter the pass does not have) is said in the returned warnings.
	pub fn resolve(desc: &PassDesc, json: &serde_json::Map<String, Value>) -> (Self, Vec<String>) {
		let mut warnings = Vec::new();
		let mut values = BTreeMap::new();
		for param in &desc.params {
			let value = json.get(param.name.as_ref()).map_or_else(
				|| param.default.clone(),
				|json| {
					let value = ParamValue::from_json(&param.ty, json).map(|value| param.clamp(value));
					value.unwrap_or_else(|| {
						warnings.push(format!(
							"the parameter {:?} is {json}, not a {:?}: the default is used",
							param.name, param.ty
						));
						param.default.clone()
					})
				},
			);
			values.insert(param.name.to_string(), value);
		}
		for name in json.keys() {
			if desc.param_named(name).is_none() {
				warnings.push(format!("{} has no parameter {name:?}", desc.type_name));
			}
		}
		(Self { values }, warnings)
	}

	pub fn get(&self, name: &str) -> Option<&ParamValue> {
		self.values.get(name)
	}

	pub fn bool(&self, name: &str) -> Option<bool> {
		match self.get(name)? {
			ParamValue::Bool(value) => Some(*value),
			_ => None,
		}
	}

	pub fn int(&self, name: &str) -> Option<i64> {
		match self.get(name)? {
			ParamValue::Int(value) => Some(*value),
			_ => None,
		}
	}

	pub fn float(&self, name: &str) -> Option<f32> {
		match self.get(name)? {
			ParamValue::Float(value) => Some(*value),
			_ => None,
		}
	}

	pub fn vec2(&self, name: &str) -> Option<[f32; 2]> {
		match self.get(name)? {
			ParamValue::Vec2(value) => Some(*value),
			_ => None,
		}
	}

	/// A `Vec4` or a `Color`.
	pub fn vec4(&self, name: &str) -> Option<[f32; 4]> {
		match self.get(name)? {
			ParamValue::Vec4(value) | ParamValue::Color(value) => Some(*value),
			_ => None,
		}
	}

	/// An `Enum` or a `String`.
	pub fn str(&self, name: &str) -> Option<&str> {
		match self.get(name)? {
			ParamValue::Enum(value) | ParamValue::String(value) => Some(value),
			_ => None,
		}
	}

	/// Sets a value (the editor, a script). It is not checked against the
	/// description.
	pub fn set(&mut self, name: impl Into<String>, value: ParamValue) {
		self.values.insert(name.into(), value);
	}
}

/// What a type of pass is: kept in the [`PassRegistry`], so the editor needs
/// no instance of it.
#[derive(Clone, Debug, PartialEq)]
pub struct PassDesc {
	/// The key in the file of a graph (`zenith.opaque`): never renamed.
	pub type_name: Name,
	pub display_name: Name,
	pub category: PassCategory,
	pub inputs: Vec<InputDesc>,
	pub outputs: Vec<OutputDesc>,
	pub params: Vec<ParamDesc>,
	pub tooltip: Name,
}

impl PassDesc {
	/// A pass with no slots and no parameters yet.
	pub fn new(type_name: impl Into<Name>, display_name: impl Into<Name>, category: PassCategory) -> Self {
		Self {
			type_name: type_name.into(),
			display_name: display_name.into(),
			category,
			inputs: Vec::new(),
			outputs: Vec::new(),
			params: Vec::new(),
			tooltip: Cow::Borrowed(""),
		}
	}

	#[must_use]
	pub fn input(mut self, input: InputDesc) -> Self {
		self.inputs.push(input);
		self
	}

	#[must_use]
	pub fn output(mut self, output: OutputDesc) -> Self {
		self.outputs.push(output);
		self
	}

	#[must_use]
	pub fn param(mut self, param: ParamDesc) -> Self {
		self.params.push(param);
		self
	}

	pub fn input_named(&self, name: &str) -> Option<&InputDesc> {
		self.inputs.iter().find(|input| input.name == name)
	}

	pub fn output_named(&self, name: &str) -> Option<&OutputDesc> {
		self.outputs.iter().find(|output| output.name == name)
	}

	pub fn param_named(&self, name: &str) -> Option<&ParamDesc> {
		self.params.iter().find(|param| param.name == name)
	}
}

/// What `prepare` decides for this view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassAction {
	Run,
	/// Nothing to do (an empty list). A texture the pass creates is cleared
	/// by the graph, so what comes after reads a clean one.
	Skip,
}

/// A pass of the render graph.
///
/// One instance serves every view of its graph in a frame: what belongs to a
/// view goes in [`PrepareContext::view_state`], not in the fields.
pub trait RenderPass: Send + Sync + 'static {
	/// What the type is.
	fn describe() -> PassDesc
	where
		Self: Sized;

	/// The parameters, on creation and on every edit in the editor.
	///
	/// # Errors
	/// Values the pass cannot work with: the graph does not compile.
	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		let _ = params;
		Ok(())
	}

	/// Once, before the first frame of a compiled graph: layouts, shaders.
	///
	/// # Errors
	/// The pass cannot work: it is left out of every frame, with an error in
	/// the log.
	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		let _ = ctx;
		Ok(())
	}

	/// Every frame for every view, before `exec`: ask for pipelines, update
	/// buffers of its own, publish data.
	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let _ = ctx;
		PassAction::Run
	}

	/// Records commands, and nothing else: no resource is made here.
	///
	/// # Errors
	/// The pass could not draw: the error is logged once and the frame goes
	/// on.
	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()>;
}

/// Makes a new instance of a pass.
pub type PassFactory = Arc<dyn Fn() -> Box<dyn RenderPass> + Send + Sync>;

/// The types of passes a graph can use, by `type_name`. The engine registers
/// its own; the editor and the scripts add theirs.
#[derive(Clone, Default)]
pub struct PassRegistry {
	passes: BTreeMap<Name, (Arc<PassDesc>, PassFactory)>,
}

impl PassRegistry {
	pub fn new() -> Self {
		Self::default()
	}

	/// Registers `P`. A type of the same name is replaced.
	pub fn register<P: RenderPass + Default>(&mut self) {
		self.register_with(P::describe(), Arc::new(|| Box::new(P::default())));
	}

	/// Registers a pass described at run time (a script). A type of the same
	/// name is replaced.
	pub fn register_with(&mut self, desc: PassDesc, factory: PassFactory) {
		if self.passes.contains_key(&desc.type_name) {
			zenith_log::info!("The pass {} is registered again: the new one is used", desc.type_name);
		}
		self.passes.insert(desc.type_name.clone(), (Arc::new(desc), factory));
	}

	pub fn get(&self, type_name: &str) -> Option<&Arc<PassDesc>> {
		self.passes.get(type_name).map(|(desc, _)| desc)
	}

	/// A new instance of `type_name`.
	pub fn make(&self, type_name: &str) -> Option<Box<dyn RenderPass>> {
		self.passes.get(type_name).map(|(_, factory)| factory())
	}

	/// Every type, by name.
	pub fn iter(&self) -> impl Iterator<Item = &PassDesc> {
		self.passes.values().map(|(desc, _)| desc.as_ref())
	}

	pub fn len(&self) -> usize {
		self.passes.len()
	}

	pub fn is_empty(&self) -> bool {
		self.passes.is_empty()
	}
}

impl std::fmt::Debug for PassRegistry {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_list().entries(self.passes.keys()).finish()
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn desc() -> PassDesc {
		PassDesc::new("test.tonemap", "Tonemap", PassCategory::Post)
			.param(ParamDesc::float("exposure", 1.0).range(0.0, 10.0))
			.param(ParamDesc::enumeration("operator", &["aces", "reinhard"], "aces"))
			.param(ParamDesc::color("tint", [1.0; 4]))
	}

	fn object(json: &Value) -> serde_json::Map<String, Value> {
		json.as_object().cloned().unwrap_or_default()
	}

	#[test]
	fn parameters_come_from_json_with_defaults_and_ranges() {
		let json = json!({ "exposure": 40, "operator": "reinhard" });
		let (params, warnings) = ParamMap::resolve(&desc(), &object(&json));
		assert_eq!(warnings, Vec::<String>::new());
		assert_eq!(params.float("exposure"), Some(10.0), "clamped");
		assert_eq!(params.str("operator"), Some("reinhard"));
		assert_eq!(params.vec4("tint"), Some([1.0; 4]), "the default");
	}

	#[test]
	fn a_wrong_parameter_is_its_default_and_a_warning() {
		let json = json!({ "operator": "filmic", "tint": [1, 0], "shiny": true });
		let (params, warnings) = ParamMap::resolve(&desc(), &object(&json));
		assert_eq!(params.str("operator"), Some("aces"));
		assert_eq!(params.vec4("tint"), Some([1.0; 4]));
		assert_eq!(warnings.len(), 3, "{warnings:?}");
	}

	#[test]
	fn a_value_goes_to_json_and_back() {
		for (ty, value) in [
			(ParamType::Bool, ParamValue::Bool(true)),
			(ParamType::Int, ParamValue::Int(-3)),
			(ParamType::Float, ParamValue::Float(0.5)),
			(ParamType::Vec3, ParamValue::Vec3([1.0, 2.0, 3.0])),
			(ParamType::Color, ParamValue::Color([0.25; 4])),
			(ParamType::String, ParamValue::String("x".into())),
		] {
			assert_eq!(ParamValue::from_json(&ty, &value.to_json()), Some(value));
		}
	}
}
