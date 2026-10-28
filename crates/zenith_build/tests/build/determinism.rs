use std::path::Path;

use zenith_build::{BuildFormat, Builder};
use zenith_filesystem::Project;

use crate::common::{TestResult, key, random_bytes, sha256, standard_project, tree_hashes, write};

fn archive(root: &Path, out: &Path, encrypted: bool) -> TestResult {
	let mut b = Builder::new().path(root).output(out).zstd_compression(3);
	if encrypted {
		b = b.encryption_key(key(5));
	}
	b.build()?;
	Ok(())
}

fn folder(root: &Path, out: &Path, encrypted: bool) -> TestResult {
	let mut b = Builder::new()
		.path(root)
		.output(out)
		.format(BuildFormat::Fs)
		.zstd_compression(3);
	if encrypted {
		b = b.encryption_key(key(5));
	}
	b.build()?;
	Ok(())
}

#[test]
fn two_archive_builds_are_byte_identical() -> TestResult {
	let (tmp, root) = standard_project()?;
	for encrypted in [false, true] {
		let (a, b) = (tmp.path().join("a.zenithbuild"), tmp.path().join("b.zenithbuild"));
		archive(&root, &a, encrypted)?;
		archive(&root, &b, encrypted)?;
		assert_eq!(
			std::fs::read(&a)?,
			std::fs::read(&b)?,
			"whole file, encrypted: {encrypted}"
		);
		archive(&root, &a, encrypted)?;
		assert_eq!(
			std::fs::read(&a)?,
			std::fs::read(&b)?,
			"rebuilding over the old file, encrypted: {encrypted}"
		);
	}
	Ok(())
}

#[test]
fn two_folder_builds_are_identical_file_by_file() -> TestResult {
	let (tmp, root) = standard_project()?;
	for encrypted in [false, true] {
		let (a, b) = (
			tmp.path().join(format!("a{encrypted}")),
			tmp.path().join(format!("b{encrypted}")),
		);
		folder(&root, &a, encrypted)?;
		folder(&root, &b, encrypted)?;
		assert_eq!(
			tree_hashes(&a),
			tree_hashes(&b),
			"all files and the index, encrypted: {encrypted}"
		);
		folder(&root, &a, encrypted)?;
		assert_eq!(
			tree_hashes(&a),
			tree_hashes(&b),
			"rebuilding into the same folder, encrypted: {encrypted}"
		);
	}
	Ok(())
}

#[test]
fn a_changed_included_file_changes_the_archive() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("x.zenithbuild");
	archive(&root, &out, false)?;
	let before = std::fs::read(&out)?;
	write(&root, "Assets/Worlds/A.zenith", b"world a, edited")?;
	archive(&root, &out, false)?;
	assert_ne!(std::fs::read(&out)?, before, "the archive reflects the edit");
	Ok(())
}

#[test]
fn a_changed_excluded_file_changes_nothing() -> TestResult {
	let (tmp, root) = standard_project()?;
	for format in [BuildFormat::Archive, BuildFormat::Fs] {
		let out = tmp.path().join(format!("out-{format:?}"));
		let build = |out: &Path| -> TestResult {
			Builder::new()
				.path(&root)
				.output(out)
				.format(format)
				.zstd_compression(3)
				.exclude("Assets/Scripts/**")
				.build()?;
			Ok(())
		};
		build(&out)?;
		let before = tree_hashes(&out);
		let before_file = if format == BuildFormat::Archive {
			std::fs::read(&out)?
		} else {
			Vec::new()
		};

		write(&root, "Assets/Scripts/lib.rs", b"fn changed() {}\n")?;
		write(&root, "Temp/junk.txt", b"changed junk")?;
		write(&root, "Docs/readme.txt", b"changed docs")?;
		build(&out)?;
		assert_eq!(tree_hashes(&out), before, "folder contents, {format:?}");
		if format == BuildFormat::Archive {
			assert_eq!(std::fs::read(&out)?, before_file, "archive bytes");
		}
		write(&root, "Assets/Scripts/lib.rs", b"fn main() {}\n")?;
	}
	Ok(())
}

// The hashes below pin the on-disk output of a fixed project and key. The
// content is incompressible random data, which zstd stores as raw blocks, so
// they do not depend on the zstd version; the manifest is the only compressed
// text and the lock file fixes zstd. If a hash changes, the output changed:
// that needs a conscious decision (and usually a format version bump).
const GOLDEN_ARCHIVE_PLAIN: &str = "3af809d684d1b86e64a1bc81105b521d398a2252579b3d01d137eee3a8df25d6";
const GOLDEN_ARCHIVE_ENCRYPTED: &str = "33218e741c98955bce6bb6612a2b88ed7f3451ed2fcd804c8de19d5da2b9cf39";
const GOLDEN_FOLDER_PLAIN: &str = "de054c7d5e53308dbe3b01156bfde26f06db02a4be68feff82b29aa9f901c1d5";
const GOLDEN_FOLDER_ENCRYPTED: &str = "a7dac4c5f9a8dbdb8bab8418f1f76aabd9c13c4014dbffefef2eca6142f0a0f3";

const GOLDEN_MANIFEST: &str =
	"[project]\nname = \"Golden\"\nproject_version = \"0.1.0\"\nengine_version = \"^0.1.0\"\nversion = 1\n";

fn golden_project() -> Result<(tempfile::TempDir, std::path::PathBuf), Box<dyn std::error::Error>> {
	let tmp = tempfile::tempdir()?;
	let root = tmp.path().join("Golden");
	Project::create(&root)?;
	// Project::create writes this workspace's version into the manifest as the
	// engine version. The golden output must not change when the workspace
	// version is raised, so the manifest is fixed here.
	write(&root, "Golden.zenithproject", GOLDEN_MANIFEST.as_bytes())?;
	write(&root, "Assets/data.bin", &random_bytes(5000, 11))?;
	write(&root, "Assets/Worlds/w.bin", &random_bytes(300, 12))?;
	write(&root, "Settings/s.bin", &random_bytes(70, 13))?;
	write(&root, "Packages/p.bin", &random_bytes(9000, 14))?;
	Ok((tmp, root))
}

fn folder_hash(dir: &Path) -> String {
	let mut all = String::new();
	for (path, hash) in tree_hashes(dir) {
		all.push_str(&path);
		all.push(' ');
		all.push_str(&hash);
		all.push('\n');
	}
	sha256(all.as_bytes())
}

#[test]
fn golden_hashes() -> TestResult {
	let (tmp, root) = golden_project()?;
	let build = |format: BuildFormat, encrypted: bool| -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
		let out = tmp.path().join(format!("out-{format:?}-{encrypted}"));
		let mut b = Builder::new()
			.path(&root)
			.output(&out)
			.format(format)
			.zstd_compression(3);
		if encrypted {
			b = b.encryption_key(key(7));
		}
		b.build()?;
		Ok(out)
	};
	let hash_file = |p: &Path| -> Result<String, Box<dyn std::error::Error>> { Ok(sha256(&std::fs::read(p)?)) };

	assert_eq!(
		hash_file(&build(BuildFormat::Archive, false)?)?,
		GOLDEN_ARCHIVE_PLAIN,
		"plain archive"
	);
	assert_eq!(
		hash_file(&build(BuildFormat::Archive, true)?)?,
		GOLDEN_ARCHIVE_ENCRYPTED,
		"encrypted archive"
	);
	assert_eq!(
		folder_hash(&build(BuildFormat::Fs, false)?),
		GOLDEN_FOLDER_PLAIN,
		"plain folder"
	);
	assert_eq!(
		folder_hash(&build(BuildFormat::Fs, true)?),
		GOLDEN_FOLDER_ENCRYPTED,
		"encrypted folder"
	);
	Ok(())
}

#[test]
fn the_report_carries_the_id_of_the_build_and_the_id_pins_it() -> TestResult {
	use zenith_filesystem::OpenOptions;

	for (format, encrypted) in [
		(BuildFormat::Archive, false),
		(BuildFormat::Archive, true),
		(BuildFormat::Fs, false),
		(BuildFormat::Fs, true),
	] {
		let (tmp, root) = golden_project()?;
		let build = |name: &str| -> Result<_, Box<dyn std::error::Error>> {
			let mut b = Builder::new()
				.path(&root)
				.output(tmp.path().join(name))
				.format(format)
				.zstd_compression(3);
			if encrypted {
				b = b.encryption_key(key(7));
			}
			Ok(b.build()?)
		};
		let open = |report: &zenith_build::BuildReport, expected: Option<zenith_build::BuildId>| {
			let mut options = OpenOptions::new();
			if encrypted {
				options = options.encryption_key(key(7));
			}
			if let Some(id) = expected {
				options = options.expected_build_id(id);
			}
			options.open(&report.output)
		};

		let first = build("one")?;
		assert_eq!(
			open(&first, None)?.build_id(),
			Some(first.build_id),
			"the report names the build that is on disk ({format:?}, encrypted: {encrypted})"
		);
		assert_eq!(
			build("again")?.build_id,
			first.build_id,
			"the same project gives the same id"
		);

		// The next release.
		write(&root, "Assets/data.bin", &random_bytes(5000, 99))?;
		let second = build("two")?;
		assert_ne!(second.build_id, first.build_id, "another content, another id");

		assert!(
			open(&second, Some(second.build_id)).is_ok(),
			"the release that was expected"
		);
		assert!(
			open(&first, Some(second.build_id)).is_err(),
			"the first release, complete and valid, is not the one that was expected"
		);
	}
	Ok(())
}
