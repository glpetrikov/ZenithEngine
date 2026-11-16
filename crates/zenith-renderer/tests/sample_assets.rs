//! The assets of the sample project (`Sandbox/Test`) are found and read.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used, clippy::panic)]

use zenith_filesystem::{Project, ZPath};
use zenith_renderer::assets::{
	AssetKind, AssetRegistry,
	import::image::{Pixels, TextureUsage},
	load,
};

fn sample() -> (Project, AssetRegistry) {
	let project =
		Project::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Sandbox/Test")).expect("open the sample project");
	let registry = AssetRegistry::scan(&project).expect("scan its assets");
	(project, registry)
}

fn id_of(registry: &AssetRegistry, path: &str) -> zenith_renderer::assets::AssetId {
	registry
		.id_of(&ZPath::new(path).expect("a valid path"))
		.unwrap_or_else(|| panic!("{path} is not an asset of the sample: {:?}", registry.problems()))
}

#[test]
fn the_cube_is_read_with_its_material_and_textures() {
	let (project, registry) = sample();
	assert_eq!(registry.problems(), &[], "the sample has no broken assets");

	let cube = load::mesh(
		&project,
		&registry,
		id_of(&registry, "Assets://Models/Cube/cube.zenithmodel"),
	)
	.expect("read the cube");
	let [mesh] = cube.submeshes.as_slice() else {
		panic!("one mesh, not {}", cube.submeshes.len());
	};
	assert_eq!(mesh.indices.len(), 36, "six faces of two triangles");

	let material_id = mesh.material.expect("the cube has a material");
	assert_eq!(registry.get(material_id).map(|e| e.kind), Some(AssetKind::Material));
	let material = load::material(&project, &registry, material_id).expect("read the material");

	let diffuse =
		load::image(&project, &registry, material.base_color_texture.expect("a texture")).expect("read the texture");
	assert_eq!(diffuse.usage, TextureUsage::Color);
	let normal =
		load::image(&project, &registry, material.normal_texture.expect("a normal map")).expect("read the normal map");
	assert_eq!(normal.usage, TextureUsage::Normal);
}

#[test]
fn the_sky_is_read_as_floats() {
	let (project, registry) = sample();
	let sky = load::image(
		&project,
		&registry,
		id_of(&registry, "Assets://Sky/pure-sky.zenithtexture"),
	)
	.expect("read the sky");
	assert_eq!(sky.usage, TextureUsage::Hdr);
	let Pixels::Rgba32Float(pixels) = sky.pixels else {
		panic!("an HDR sky is floats");
	};
	assert!(pixels.iter().any(|&value| value > 1.0), "brighter than white somewhere");
}
