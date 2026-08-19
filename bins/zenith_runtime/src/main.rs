use std::path::Path;

use zenith_components::Transform;
use zenith_error::{IntoPubResult, WrapErr};
#[allow(clippy::wildcard_imports)]
use zenith_log::*;
use zenith_world::World;

#[instrument]
fn main() -> zenith_error::ZPubResult<()> {
	zenith_error::install().into_pub_result()?;
	let _log_guard = zenith_log::init("Logs", zenith_log::Rotation::MINUTELY, 10)?;

	info!("runtime starting up");
	warn!(reason = "test warning", "something to look at");

	{
		let mut world: World = World::new("Test");
		println!(
			"entity count before creation entity: {}",
			world.world().iter_entities().count()
		);
		let entity = world.create_entity("TestEntity");

		let _ = world.add_component(entity, Transform::default()).wrap_err("");
		world.get_component_mut::<Transform>(entity).expect(":(").position = zenith_types::Vec3::new(10.0, 10.0, 12.0);

		println!("entity count before save: {}", world.world().iter_entities().count());

		let _ = world.save(Path::new("Sandbox"), "Test");
	}
	let world: World = World::from_path("Sandbox/Test.zenith").expect(":(");
	println!("{:?}", world.registry().registered_types().collect::<String>());

	// Force an error through the eyre path to see wrap_err/downcast working.
	std::thread::spawn(|| {
		if let Err(report) = try_something() {
			error!(error = %report, "something failed");
		}
	});

	info!("runtime shutting down");

	panic!("Fatal Error")
}
#[instrument]
fn try_something() -> zenith_error::ZResult<()> {
	use zenith_error::WrapErr;
	std::fs::read_to_string("this_file_does_not_exist.txt").wrap_err("failed to read config file")?;
	Ok(())
}
