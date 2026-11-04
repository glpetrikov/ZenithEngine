use std::sync::Arc;

use zenith_archive::EncryptionKey;
use zenith_filesystem::{MemStore, Project};

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn project() -> Result<(tempfile::TempDir, Project), Box<dyn std::error::Error>> {
	let tmp = tempfile::tempdir()?;
	let project = Project::create(tmp.path().join("Game"))?;
	project.set_user_data(Arc::new(MemStore::new()));
	Ok((tmp, project))
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Stats {
	hp: i32,
	items: Vec<String>,
	position: (f32, f32),
}

#[test]
fn every_kind_of_value_comes_back() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let mut saver = saves.saver("slot1", SaveMeta::new("sections", 1));
	saver.begin("player")?;
	saver.save_bool("alive", true)?;
	saver.save_i32("score", -32)?;
	saver.save_u32("coins", 4_000_000_000)?;
	saver.save_i64("big", i64::MIN)?;
	saver.save_u64("huge", u64::try_from(i64::MAX)?)?;
	saver.save_f32("speed", 1.5)?;
	saver.save_f64("pi", std::f64::consts::PI)?;
	saver.save_string("name", "Ada \"the\" Лавлейс\n")?;
	saver.save_bytes("blob", &[0, 1, 2, 254, 255])?;
	saver.end()?;
	let info = saver.finish()?;
	assert_eq!(info.meta.title, "sections", "the slot is written");

	let loader = saves.loader("slot1")?;
	assert_eq!(loader.sections(), ["player"], "the section is listed");
	assert!(loader.has_section("player") && !loader.has_section("world"));
	let player = loader.section("player")?;
	assert_eq!(player.name(), "player");
	assert_eq!(player.get_version(), 1, "begin means version 1");
	assert!(player.load_bool("alive")?, "bool");
	assert_eq!(player.load_i32("score")?, -32, "i32");
	assert_eq!(player.load_u32("coins")?, 4_000_000_000, "u32");
	assert_eq!(player.load_i64("big")?, i64::MIN, "i64");
	assert_eq!(player.load_u64("huge")?, u64::try_from(i64::MAX)?, "u64");
	assert!((player.load_f32("speed")? - 1.5).abs() < f32::EPSILON, "f32");
	assert!(
		(player.load_f64("pi")? - std::f64::consts::PI).abs() < f64::EPSILON,
		"f64"
	);
	assert_eq!(player.load_string("name")?, "Ada \"the\" Лавлейс\n", "text");
	assert_eq!(player.load_bytes("blob")?, [0, 1, 2, 254, 255], "bytes");
	assert_eq!(player.load_i32_or("lives", 3)?, 3, "a default for a missing value");
	assert_eq!(player.load_i32_or("score", 3)?, -32, "and not for one that is there");
	assert_eq!(player.load_string_or("nick", "none")?, "none");
	assert_eq!(player.load_bytes_or("none", &[9])?, [9]);
	assert_eq!(player.keys().len(), 9, "the keys");
	Ok(())
}

#[test]
fn the_version_of_a_section_is_kept() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let mut saver = saves.saver("slot1", SaveMeta::new("v", 1));
	saver.begin_versioned("world", 7)?;
	saver.save_i32("seed", 1)?;
	saver.end()?;
	saver.begin("player")?;
	saver.end()?;
	saver.finish()?;
	let loader = saves.loader("slot1")?;
	assert_eq!(loader.section("world")?.get_version(), 7);
	assert_eq!(loader.section("player")?.get_version(), 1, "an empty section is fine");
	Ok(())
}

#[test]
fn wrong_use_is_an_error_and_nothing_is_written() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let mut saver = saves.saver("slot1", SaveMeta::new("x", 1));
	assert!(
		matches!(saver.save_i32("a", 1), Err(ZenithError::InvalidSaveData(_))),
		"a value without a section"
	);
	assert!(saver.end().is_err(), "end without begin");
	assert!(saver.begin_versioned("p", 0).is_err(), "version 0");
	for bad in ["", "a b", "a.b", "a/b", "ü", &"n".repeat(65)] {
		assert!(saver.begin(bad).is_err(), "{bad:?} is not a section name");
	}
	saver.begin("p")?;
	assert!(saver.begin("q").is_err(), "a second begin");
	assert!(saver.save_i32("bad key", 1).is_err(), "a bad key");
	saver.save_i32("a", 1)?;
	assert!(saver.save_i32("a", 2).is_err(), "the same key twice");
	assert!(
		matches!(saver.save_u64("big", u64::MAX), Err(ZenithError::InvalidSaveData(_))),
		"a u64 that does not fit"
	);
	saver.end()?;
	assert!(saver.begin("p").is_err(), "the same section twice");
	saver.begin("open")?;
	assert!(
		matches!(saver.finish(), Err(ZenithError::InvalidSaveData(_))),
		"finish with a section open"
	);
	assert!(!saves.exists("slot1")?, "so nothing was written");
	Ok(())
}

#[test]
fn a_failed_save_leaves_the_slot_as_it_was() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let mut first = saves.saver("slot1", SaveMeta::new("good", 1));
	first.begin("p")?;
	first.save_i32("a", 1)?;
	first.end()?;
	first.finish()?;

	let mut second = saves.saver("slot1", SaveMeta::new("never", 1));
	second.begin("p")?;
	second.save_i32("a", 2)?;
	assert!(second.finish().is_err(), "unfinished section");
	assert_eq!(saves.loader("slot1")?.section("p")?.load_i32("a")?, 1);
	assert_eq!(saves.info("slot1")?.meta.title, "good");
	Ok(())
}

#[test]
fn what_is_not_asked_for_is_an_error_that_says_what() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let mut saver = saves.saver("slot1", SaveMeta::new("x", 1));
	saver.begin("p")?;
	saver.save_i64("n", i64::from(i32::MAX) + 1)?;
	saver.save_string("s", "not bytes!")?;
	saver.save_f64("f", 1.0)?;
	saver.save_i32("i", 1)?;
	saver.end()?;
	saver.finish()?;
	let loader = saves.loader("slot1")?;
	let p = loader.section("p")?;
	assert!(
		matches!(p.load_i32("n"), Err(ZenithError::SaveWrongType(_))),
		"does not fit an i32"
	);
	assert!(matches!(p.load_u32("i"), Ok(1)), "fits a u32");
	assert!(
		matches!(p.load_i32("s"), Err(ZenithError::SaveWrongType(_))),
		"text is not a number"
	);
	assert!(
		matches!(p.load_string("i"), Err(ZenithError::SaveWrongType(_))),
		"a number is not text"
	);
	assert!(
		matches!(p.load_bytes("s"), Err(ZenithError::SaveWrongType(_))),
		"text that is not base64"
	);
	assert!(
		(p.load_f64("i")? - 1.0).abs() < f64::EPSILON,
		"an integer is read as a float, for saves edited by hand"
	);
	assert!(
		matches!(p.load_f64("s"), Err(ZenithError::SaveWrongType(_))),
		"text is not a float"
	);
	assert!(matches!(p.load_bool("i"), Err(ZenithError::SaveWrongType(_))));
	assert!(
		matches!(p.load_i32_or("s", 0), Err(ZenithError::SaveWrongType(_))),
		"a default does not hide a wrong type"
	);
	let error = p.load_i32("missing").err();
	assert!(
		matches!(&error, Some(ZenithError::SaveKeyNotFound(m)) if m == "p.missing"),
		"{error:?}"
	);
	assert!(
		matches!(loader.section("nope"), Err(ZenithError::SaveSectionNotFound(_))),
		"no such section"
	);
	assert!(loader.section("bad name").is_err(), "a bad name is asked for");
	Ok(())
}

#[test]
fn serde_values_and_whole_sections() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let stats = Stats {
		hp: 40,
		items: vec!["key".into(), "map".into()],
		position: (1.5, -2.0),
	};
	let mut saver = saves.saver("slot1", SaveMeta::new("serde", 1));
	saver.begin("player")?;
	saver.save("stats", &stats)?;
	saver.save("tags", &["a", "b"])?;
	saver.end()?;
	saver.save_section("whole", 3, &stats)?;
	assert!(
		saver.save_section("not-a-table", 1, &5).is_err(),
		"a section needs fields"
	);
	saver.finish()?;

	let loader = saves.loader("slot1")?;
	let player = loader.section("player")?;
	assert_eq!(player.load::<Stats>("stats")?, stats, "a struct as a value");
	assert_eq!(player.load::<Vec<String>>("tags")?, ["a", "b"], "a list");
	assert!(
		matches!(player.load::<Vec<i32>>("stats"), Err(ZenithError::SaveWrongType(_))),
		"the wrong shape"
	);
	let whole = loader.section("whole")?;
	assert_eq!(whole.get_version(), 3);
	assert_eq!(whole.load_all::<Stats>()?, stats, "a section as a struct");
	assert_eq!(whole.load_i32("hp")?, 40, "and as values");
	Ok(())
}

#[test]
fn a_damaged_section_is_named_and_the_others_still_read() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let save = Save::new(SaveMeta::new("x", 1))
		.with_entry("Sections/good.toml", "[section]\nversion = 2\n[values]\nn = 5\n")
		.with_entry("Sections/garbled.toml", "this is = not [ toml")
		.with_entry("Sections/zero.toml", "[section]\nversion = 0\n");
	saves.save("slot1", &save)?;

	let loader = saves.loader("slot1")?;
	assert_eq!(loader.section("good")?.load_i32("n")?, 5, "the good one reads");
	for bad in ["garbled", "zero"] {
		let error = loader.section(bad).err();
		assert!(
			matches!(&error, Some(ZenithError::SaveDamaged(m)) if m.contains(bad) && m.contains("slot1")),
			"{bad}: {error:?}"
		);
	}
	Ok(())
}

#[test]
fn sections_are_listed_in_order_and_strange_names_are_left_out() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let toml = "[section]\nversion = 1\n";
	let save = Save::new(SaveMeta::new("x", 1))
		.with_entry("Sections/zeta.toml", toml)
		.with_entry("Sections/a/b.toml", toml)
		.with_entry("Sections/alpha.toml", toml)
		.with_entry("Sections/not a name.toml", toml)
		.with_entry("Sections/readme.txt", "x");
	saves.save("slot1", &save)?;
	assert_eq!(saves.loader("slot1")?.sections(), ["alpha", "zeta"]);
	Ok(())
}

#[test]
fn the_folder_of_the_sections_is_not_a_place_for_files_in_any_case() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let save = Save::new(SaveMeta::new("x", 1))
		.with_entry("sections/x", vec![1])
		.with_entry("Sections/y.toml", "[section]\nversion = 1\n")
		.with_entry("other.bin", vec![2]);
	saves.save("slot1", &save)?;
	let loader = saves.loader("slot1")?;
	assert_eq!(loader.entries(), ["other.bin"], "neither is listed as a plain file");
	assert!(loader.entry("sections/x").is_err() && loader.entry("Sections/y.toml").is_err());
	assert_eq!(loader.entry("other.bin")?, [2]);
	Ok(())
}

#[test]
fn other_files_live_beside_the_sections() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	let mut saver = saves.saver("slot1", SaveMeta::new("x", 1));
	saver.entry("world.zenith", vec![1, 2, 3])?;
	assert!(saver.entry("world.zenith", vec![4]).is_err(), "twice");
	assert!(
		saver.entry("Sections/player.toml", vec![4]).is_err(),
		"Sections/ is reserved"
	);
	assert!(saver.entry("Save.zenithproject", vec![4]).is_err(), "so is the header");
	for sneaky in [
		"./Sections/x",
		"Sections//x",
		"sections/x.toml",
		"SECTIONS/x.toml",
		"a/../Sections/x",
		"./Save.zenithproject",
		"save.ZenithProject",
	] {
		assert!(
			saver.entry(sneaky, vec![4]).is_err(),
			"{sneaky:?} is refused when writing"
		);
	}
	saver.begin("p")?;
	saver.end()?;
	saver.finish()?;
	let loader = saves.loader("slot1")?;
	assert_eq!(loader.entries(), ["world.zenith"], "the files that are not sections");
	assert_eq!(loader.entry("world.zenith")?, [1, 2, 3]);
	assert!(loader.entry("nope").is_err() && loader.entry("Save.zenithproject").is_err());
	assert_eq!(
		saves.load("slot1")?.entry("world.zenith"),
		Some(&[1u8, 2, 3][..]),
		"Saves::load sees it"
	);
	Ok(())
}

#[test]
fn the_loader_follows_the_rules_of_a_save() -> TestResult {
	let (_tmp, project) = project()?;
	let saves = Saves::new(&project);
	assert!(
		matches!(saves.loader("nope"), Err(ZenithError::SaveNotFound(_))),
		"an empty slot"
	);
	for (title, n) in [("one", 1), ("two", 2)] {
		let mut saver = saves.saver("slot1", SaveMeta::new(title, 1));
		saver.begin("p")?;
		saver.save_i32("n", n)?;
		saver.end()?;
		saver.finish()?;
	}
	let backup = saves.loader_backup("slot1")?;
	assert!(backup.from_backup(), "the backup says so");
	assert_eq!(backup.section("p")?.load_i32("n")?, 1, "with the older progress");
	assert_eq!(saves.loader("slot1")?.meta().title, "two");

	project.write(&Saves::file("slot1", crate::SAVE_EXTENSION)?, b"this is not a save")?;
	assert!(
		matches!(saves.loader("slot1"), Err(ZenithError::SaveDamaged(_))),
		"a damaged save is an error"
	);
	Ok(())
}

#[test]
fn encrypted_sections_need_the_key() -> TestResult {
	let (_tmp, project) = project()?;
	let secret = Saves::new(&project).encryption_key(EncryptionKey::new([7; 32]));
	let mut saver = secret.saver("slot1", SaveMeta::new("x", 1));
	saver.begin("p")?;
	saver.save_string("password", "hunter2hunter2")?;
	saver.end()?;
	saver.finish()?;
	assert_eq!(
		secret.loader("slot1")?.section("p")?.load_string("password")?,
		"hunter2hunter2"
	);
	let raw = project.read_bytes(&Saves::file("slot1", crate::SAVE_EXTENSION)?)?;
	assert!(!raw.windows(7).any(|w| w == b"hunter2"), "not readable in the file");
	assert!(matches!(
		Saves::new(&project).loader("slot1"),
		Err(ZenithError::ArchiveEncryptedNoKey)
	));
	Ok(())
}
