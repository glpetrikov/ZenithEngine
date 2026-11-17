//! WESL to WGSL, checked.
//!
//! The `wesl` compiler resolves the `import`s through the [`ShaderSources`]
//! (so it needs no folder: it works in an archive and in a browser), keeps
//! what the `@if` conditions ask for and strips what no entry point reaches.
//! The WGSL it gives is then checked with `naga`, so an error in a file of a
//! project is an error here, with a message, and does not reach the error
//! handler of the device (which panics).

use std::borrow::Cow;

use wesl::{
	CompileOptions,
	resolver::VirtualResolver,
	syntax::{ModulePath, PathOrigin},
};
use zenith_error::{ZResult, ZenithError};

use super::ShaderSources;

/// The path WESL imports a shader by: `Zenith/standard` is
/// `package::Zenith::standard`.
fn module_path(name: &str) -> ModulePath {
	ModulePath::new(PathOrigin::Absolute, name.split('/').map(str::to_owned).collect())
}

/// The shader `name`, with the `conditions` it is compiled with (`@if(name)`
/// in the text), as WGSL that is valid.
///
/// # Errors
/// There is no such shader, it does not compile or its WGSL is not valid. The
/// message names the file and the line when it can.
pub fn compile(name: &str, sources: &ShaderSources, conditions: &[(String, bool)]) -> ZResult<String> {
	if sources.get(name).is_none() {
		return Err(ZenithError::Other(format!("shader {name}: there is no such shader")));
	}
	let mut resolver = VirtualResolver::new();
	for (module, text) in sources.iter() {
		resolver.add_module(module_path(module), Cow::Borrowed(text));
	}
	let mut options = CompileOptions::default();
	for (condition, on) in conditions {
		options.features.set(condition, *on);
	}
	let compiled = wesl::compile(&module_path(name), &options, &resolver)
		.map_err(|e| ZenithError::Other(format!("shader {name}: {e}")))?;
	let wgsl = compiled.to_string();
	check(name, &wgsl)?;
	Ok(wgsl)
}

/// Parses and validates `wgsl` the way the device will.
///
/// A browser has no `naga` here: it checks the shader itself, and the check
/// is left to it.
///
/// # Errors
/// The message of `naga`, with the line.
#[cfg(not(target_arch = "wasm32"))]
pub fn check(name: &str, wgsl: &str) -> ZResult<()> {
	use wgpu::naga;

	let module = naga::front::wgsl::parse_str(wgsl)
		.map_err(|e| ZenithError::Other(format!("shader {name}: {}", e.emit_to_string(wgsl))))?;
	naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
		.validate(&module)
		.map_err(|e| ZenithError::Other(format!("shader {name}: {}", e.emit_to_string(wgsl))))?;
	Ok(())
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps, clippy::missing_errors_doc)]
pub fn check(_name: &str, _wgsl: &str) -> ZResult<()> {
	Ok(())
}

#[cfg(test)]
mod tests {
	#![allow(clippy::expect_used, clippy::panic)]

	use super::*;
	use crate::shaders::{ENGINE, MAGENTA, STANDARD};

	fn none() -> Vec<(String, bool)> {
		Vec::new()
	}

	#[test]
	fn every_shader_of_the_engine_compiles_and_is_valid() {
		let sources = ShaderSources::engine();
		for (name, _) in ENGINE {
			// The common module has no entry point: everything is stripped, and
			// that is fine.
			compile(name, &sources, &none()).unwrap_or_else(|e| panic!("{e}"));
		}
	}

	#[test]
	fn what_is_imported_ends_up_in_the_output() {
		let wgsl = compile(MAGENTA, &ShaderSources::engine(), &none()).expect("it compiles");
		assert!(
			wgsl.contains("view_proj"),
			"the Camera struct came from Zenith/common:\n{wgsl}"
		);
		assert!(wgsl.contains("fn vs_main"));
	}

	#[test]
	fn a_condition_picks_a_variant() {
		let mut sources = ShaderSources::engine();
		sources.set(
			"Mine/tint",
			"@fragment fn fs_main() -> @location(0) vec4<f32> {\n\
			 \t@if(red) { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }\n\
			 \t@else { return vec4<f32>(0.0, 0.0, 1.0, 1.0); }\n}\n",
		);
		let red = compile("Mine/tint", &sources, &[("red".into(), true)]).expect("red");
		let blue = compile("Mine/tint", &sources, &[("red".into(), false)]).expect("blue");
		assert!(red.contains("1.0, 0.0, 0.0") || red.contains("1f, 0f, 0f"), "{red}");
		assert_ne!(red, blue);
	}

	#[test]
	fn a_shader_that_is_not_there_or_is_broken_is_an_error_with_a_message() {
		let mut sources = ShaderSources::engine();
		let missing = compile("Mine/none", &sources, &none()).expect_err("no such shader");
		assert!(missing.to_string().contains("Mine/none"), "{missing}");

		sources.set(
			"Mine/typo",
			"@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0 }",
		);
		let broken = compile("Mine/typo", &sources, &none()).expect_err("a syntax error");
		assert!(broken.to_string().contains("Mine/typo"), "{broken}");

		// Valid to read and wrong to use: naga finds it.
		sources.set(
			"Mine/types",
			"@fragment fn fs_main() -> @location(0) vec4<f32> { return 1.0; }",
		);
		assert!(compile("Mine/types", &sources, &none()).is_err());

		sources.set(
			"Mine/import",
			"import package::Mine::absent::Thing;
@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(Thing, 0.0, 0.0, 1.0); }",
		);
		assert!(
			compile("Mine/import", &sources, &none()).is_err(),
			"an import that is not there"
		);
	}

	#[test]
	fn plain_wgsl_is_valid_wesl() {
		let mut sources = ShaderSources::engine();
		sources.set(
			"Mine/plain",
			"struct V { @location(0) p: vec3<f32> }\n@vertex fn vs_main(v: V) -> @builtin(position) vec4<f32> { return vec4<f32>(v.p, 1.0); }\n",
		);
		assert!(compile("Mine/plain", &sources, &none()).is_ok());
		let _ = STANDARD;
	}
}
