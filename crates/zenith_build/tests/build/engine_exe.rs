use zenith_archive::key_slot;
use zenith_build::{BuildFormat, Builder, RenameExe};
use zenith_error::ZenithError;

use crate::common::{TestResult, standard_project, write};

#[test]
fn the_engine_executable_is_copied_next_to_the_build() -> TestResult {
	let (tmp, root) = standard_project()?;
	let engine = tmp.path().join("engine");
	write(&engine, "zenith_engine.exe", b"MZ engine")?;
	let out = tmp.path().join("dist").join("game.zenithbuild");

	let report = Builder::new()
		.path(&root)
		.output(&out)
		.engine_exe(engine.join("zenith_engine.exe"))
		.build()?;
	let copy = tmp.path().join("dist").join("zenith_engine.exe");
	assert_eq!(report.engine_exe.as_deref(), Some(copy.as_path()), "reported");
	assert_eq!(std::fs::read(&copy)?, b"MZ engine", "same content");
	assert_eq!(
		std::fs::read(engine.join("zenith_engine.exe"))?,
		b"MZ engine",
		"the original stays"
	);
	assert!(
		std::fs::read_dir(tmp.path().join("dist"))?
			.all(|e| e.is_ok_and(|e| !e.file_name().to_string_lossy().ends_with(".zenith-tmp"))),
		"no temporary files are left"
	);

	write(&engine, "zenith_engine.exe", b"MZ newer engine")?;
	Builder::new()
		.path(&root)
		.output(&out)
		.engine_exe(engine.join("zenith_engine.exe"))
		.build()?;
	assert_eq!(
		std::fs::read(&copy)?,
		b"MZ newer engine",
		"an existing copy is replaced"
	);
	Ok(())
}

#[test]
fn the_copy_can_be_renamed_and_keeps_its_extension() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(tmp.path(), "engine/zenith_engine.exe", b"MZ")?;
	let source = tmp.path().join("engine").join("zenith_engine.exe");

	let by_project = tmp.path().join("a");
	Builder::new()
		.path(&root)
		.output(by_project.join("g.zenithbuild"))
		.engine_exe(&source)
		.rename_exe(RenameExe::ProjectName)
		.build()?;
	assert!(by_project.join("Game.exe").is_file(), "named after the project");

	let by_name = tmp.path().join("b");
	let report = Builder::new()
		.path(&root)
		.output(by_name.join("g"))
		.format(BuildFormat::Fs)
		.engine_exe(&source)
		.rename_exe(RenameExe::Name("My Game".into()))
		.build()?;
	assert!(by_name.join("My Game.exe").is_file(), "a given name; Fs builds too");
	assert_eq!(report.engine_exe, Some(by_name.join("My Game.exe")), "reported");
	assert!(
		by_name.join("g").join("build.index").is_file(),
		"the build is a folder beside it"
	);
	Ok(())
}

#[test]
fn bad_engine_options_fail_before_anything_is_written() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(tmp.path(), "engine/zenith_engine.exe", b"MZ")?;
	let source = tmp.path().join("engine").join("zenith_engine.exe");
	let out = tmp.path().join("dist").join("g.zenithbuild");
	let build = |source: &std::path::Path, rename: Option<RenameExe>| {
		let builder = Builder::new().path(&root).output(&out).engine_exe(source);
		match rename {
			Some(rename) => builder.rename_exe(rename),
			None => builder,
		}
		.build()
	};

	for (what, result) in [
		("missing", build(&tmp.path().join("nope.exe"), None)),
		("a folder", build(&tmp.path().join("engine"), None)),
		("empty name", build(&source, Some(RenameExe::Name(String::new())))),
		("separator", build(&source, Some(RenameExe::Name("a/b".into())))),
		("colon", build(&source, Some(RenameExe::Name("a:b".into())))),
		("trailing dot", build(&source, Some(RenameExe::Name("a.".into())))),
	] {
		assert!(
			matches!(result, Err(ZenithError::InvalidBuildOption(_))),
			"{what}: {result:?}"
		);
	}
	assert!(
		!out.exists() && !tmp.path().join("dist").exists(),
		"nothing was written"
	);

	Ok(())
}

/// A pretend program with a slot for the key, the way the engine's are.
fn program_with_slot(head: &[u8]) -> Vec<u8> {
	let mut bytes = head.to_vec();
	bytes.extend_from_slice(&key_slot::blank_slot());
	bytes.extend_from_slice(b"tail");
	bytes
}

/// Where the key of a copy is, if it was given one (see `key_slot`).
fn written_key(copy: &[u8], head_len: usize) -> Option<Vec<u8>> {
	let slot = copy.get(head_len..head_len + key_slot::SLOT_LEN)?;
	(slot.get(16) == Some(&1)).then(|| slot.get(17..).map(<[u8]>::to_vec))?
}

#[test]
fn the_engine_executable_is_never_compressed_or_encrypted_and_carries_the_key() -> TestResult {
	let (tmp, root) = standard_project()?;
	let head: Vec<u8> = (0..50_000u32).map(|n| (n % 251) as u8).collect();
	let exe = program_with_slot(&head);
	write(tmp.path(), "engine/zenith_engine.exe", &exe)?;
	let out = tmp.path().join("dist").join("g.zenithbuild");
	Builder::new()
		.path(&root)
		.output(&out)
		.encryption_key(crate::common::key(5))
		.zstd_compression(19)
		.engine_exe(tmp.path().join("engine").join("zenith_engine.exe"))
		.build()?;
	let copy = std::fs::read(tmp.path().join("dist").join("zenith_engine.exe"))?;
	assert_eq!(copy.len(), exe.len(), "as long as the original");
	assert_eq!(
		copy.get(..head.len()),
		Some(head.as_slice()),
		"byte for byte the original"
	);
	assert_eq!(copy.last_chunk::<4>(), Some(b"tail"), "to the end");
	assert_eq!(
		written_key(&copy, head.len()),
		Some(vec![5; 32]),
		"with the key in its slot"
	);
	assert_eq!(
		std::fs::read(tmp.path().join("engine").join("zenith_engine.exe"))?,
		exe,
		"the original keeps no key"
	);

	// A build without a key leaves the slot empty.
	Builder::new()
		.path(&root)
		.output(&out)
		.engine_exe(tmp.path().join("engine").join("zenith_engine.exe"))
		.build()?;
	assert_eq!(std::fs::read(tmp.path().join("dist").join("zenith_engine.exe"))?, exe);
	Ok(())
}

#[test]
fn a_program_without_a_key_slot_is_refused_for_an_encrypted_build() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(tmp.path(), "engine/zenith_engine.exe", b"MZ old engine")?;
	fake_pkg(tmp.path())?;
	write(tmp.path(), "pkg/zenith_runtime_bg.wasm", b"\0asm old")?;
	let out = tmp.path().join("dist").join("g.zenithbuild");
	let exe = Builder::new()
		.path(&root)
		.output(&out)
		.encryption_key(crate::common::key(5))
		.engine_exe(tmp.path().join("engine").join("zenith_engine.exe"))
		.build();
	let web = Builder::new()
		.path(&root)
		.output(&out)
		.encryption_key(crate::common::key(5))
		.web_runtime(tmp.path().join("pkg"))
		.build();
	for (what, result) in [("exe", exe), ("wasm", web)] {
		assert!(
			matches!(result, Err(ZenithError::InvalidBuildOption(_))),
			"{what}: {result:?}"
		);
	}
	assert!(!tmp.path().join("dist").exists(), "nothing was written");

	// Without a key, an old program is copied as it is.
	Builder::new()
		.path(&root)
		.output(&out)
		.engine_exe(tmp.path().join("engine").join("zenith_engine.exe"))
		.build()?;
	assert_eq!(
		std::fs::read(tmp.path().join("dist").join("zenith_engine.exe"))?,
		b"MZ old engine"
	);
	Ok(())
}

fn fake_pkg(tmp: &std::path::Path) -> TestResult {
	write(tmp, "pkg/zenith_runtime.js", b"export default async function init() {}")?;
	write(tmp, "pkg/zenith_runtime_bg.wasm", &program_with_slot(b"\0asm"))?;
	write(tmp, "pkg/zenith_runtime.d.ts", b"// types")?;
	write(tmp, "pkg/package.json", b"{}")?;
	write(tmp, "pkg/snippets/x/inline0.js", b"export {}")?;
	Ok(())
}

#[test]
fn the_web_runtime_is_copied_with_a_page() -> TestResult {
	let (tmp, root) = standard_project()?;
	fake_pkg(tmp.path())?;
	let out = tmp.path().join("web").join("game.zenithbuild");
	let report = Builder::new()
		.path(&root)
		.output(&out)
		.encryption_key(crate::common::key(5))
		.web_runtime(tmp.path().join("pkg"))
		.build()?;
	let web = tmp.path().join("web");
	let wasm = std::fs::read(web.join("pkg/zenith_runtime_bg.wasm"))?;
	assert_eq!(wasm.get(..4), Some(b"\0asm".as_slice()));
	assert_eq!(written_key(&wasm, 4), Some(vec![5; 32]), "the .wasm carries the key");
	assert!(web.join("pkg/zenith_runtime.js").is_file(), "the script");
	assert!(web.join("pkg/snippets/x/inline0.js").is_file(), "snippets come along");
	assert!(
		!web.join("pkg/zenith_runtime.d.ts").exists() && !web.join("pkg/package.json").exists(),
		"types and package.json do not"
	);
	let page = std::fs::read_to_string(web.join("index.html"))?;
	assert!(
		page.contains("./pkg/zenith_runtime.js") && page.contains("zenith-canvas"),
		"a page that starts it: {page}"
	);
	assert_eq!(report.web_files.len(), 4, "reported: {:?}", report.web_files);

	// A page of the caller's own.
	write(tmp.path(), "my.html", b"<p>mine</p>")?;
	Builder::new()
		.path(&root)
		.output(&out)
		.web_runtime(tmp.path().join("pkg"))
		.web_index(tmp.path().join("my.html"))
		.build()?;
	assert_eq!(std::fs::read(web.join("index.html"))?, b"<p>mine</p>");
	Ok(())
}

#[test]
fn a_bad_web_runtime_is_reported_before_anything_is_written() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("dist").join("g.zenithbuild");
	let missing = Builder::new()
		.path(&root)
		.output(&out)
		.web_runtime(tmp.path().join("nope"))
		.build();
	assert!(
		matches!(missing, Err(ZenithError::InvalidBuildOption(_))),
		"{missing:?}"
	);

	write(tmp.path(), "empty/readme.txt", b"x")?;
	let empty = Builder::new()
		.path(&root)
		.output(&out)
		.web_runtime(tmp.path().join("empty"))
		.build();
	assert!(
		matches!(empty, Err(ZenithError::InvalidBuildOption(_))),
		"no script: {empty:?}"
	);
	assert!(!tmp.path().join("dist").exists(), "nothing was written");
	Ok(())
}
