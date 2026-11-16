//! Tools for a project, from the command line. For now: its assets.
//!
//! ```text
//! zenith-cli [-p <project>] import <source> [--to <Assets://folder>] [--usage <color|normal|hdr|data>]
//! zenith-cli [-p <project>] export <Assets://path> [<to>]
//! zenith-cli [-p <project>] assets
//! zenith-cli [-p <project>] new-id <Assets://path>
//! ```
//!
//! Without `-p`, the project is the folder the command is run in, or the
//! nearest folder above it that is a project.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, ZPath};
use zenith_renderer::assets::{
	AssetProblem, AssetRegistry,
	import::image::TextureUsage,
	ops::{self, Imported},
};

/// Tools for a Zenith project.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
	/// The folder of the project. Without it: the project the current folder
	/// is in.
	#[arg(short, long, global = true)]
	project: Option<PathBuf>,
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	/// Imports a model or a picture into the project.
	///
	/// A model (`.obj`, `.gltf`, `.glb`) comes in with its materials and the
	/// pictures they use; a picture becomes a texture.
	Import {
		/// The file to import.
		source: PathBuf,
		/// Where in the project the assets go.
		#[arg(long, default_value = "Assets://")]
		to: String,
		/// What the pixels of a picture are; guessed from its name when not
		/// given (`normal` in it, an `.hdr`). Not for a model.
		#[arg(long, value_enum)]
		usage: Option<Usage>,
	},
	/// Writes what an asset was made from out of the project: the source of a
	/// model (a glTF comes out as `.glb`) or a texture, the file of a material.
	Export {
		/// The asset, as `Assets://...`.
		asset: String,
		/// The file to write, or a folder to write it in. Without it: the
		/// current folder.
		to: Option<PathBuf>,
	},
	/// Lists the assets of the project, and what is wrong with them.
	Assets,
	/// Gives an asset an id of its own (a copy made outside the editor).
	NewId {
		/// The asset, as `Assets://...`.
		path: String,
	},
}

#[derive(Clone, Copy, ValueEnum)]
enum Usage {
	Color,
	Normal,
	Hdr,
	Data,
}

impl From<Usage> for TextureUsage {
	fn from(usage: Usage) -> Self {
		match usage {
			Usage::Color => Self::Color,
			Usage::Normal => Self::Normal,
			Usage::Hdr => Self::Hdr,
			Usage::Data => Self::Data,
		}
	}
}

fn main() {
	if let Err(e) = run(Cli::parse()) {
		eprintln!("error: {e}");
		std::process::exit(1);
	}
}

/// The project of `-p`, or the one the current folder is in.
fn open_project(given: Option<PathBuf>) -> ZResult<Project> {
	let folder = if let Some(folder) = given {
		folder
	} else {
		let here = std::env::current_dir()?;
		Project::find_folder(&here).ok_or_else(|| {
			ZenithError::Other(format!(
				"{} is not in a project: run this in one, or give it with --project",
				here.display()
			))
		})?
	};
	Project::open(folder)
}

fn run(cli: Cli) -> ZResult<()> {
	let project = open_project(cli.project)?;
	match cli.command {
		Command::Import { source, to, usage } => import(&project, &source, &ZPath::new(&to)?, usage),
		Command::Export { asset, to } => export(&project, &ZPath::new(&asset)?, to),
		Command::Assets => list(&project),
		Command::NewId { path } => {
			let mut registry = AssetRegistry::scan(&project)?;
			let id = ops::give_new_id(&project, &mut registry, &ZPath::new(&path)?)?;
			println!("{path} is now {id}");
			Ok(())
		}
	}
}

fn import(project: &Project, source: &Path, folder: &ZPath, usage: Option<Usage>) -> ZResult<()> {
	let mut registry = AssetRegistry::scan(project)?;
	let is_model = source
		.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(ops::is_model_source);
	let made = if is_model {
		if usage.is_some() {
			return Err(ZenithError::Other("--usage is for a picture, not a model".into()));
		}
		ops::import_model(project, &mut registry, source, folder)?
	} else {
		vec![ops::import_texture(
			project,
			&mut registry,
			source,
			folder,
			usage.map(TextureUsage::from),
		)?]
	};
	for Imported { id, kind, path } in made {
		println!("{:<9} {id}  {path}", format!("{kind:?}"));
	}
	Ok(())
}

fn export(project: &Project, asset: &ZPath, to: Option<PathBuf>) -> ZResult<()> {
	let to = to.unwrap_or_else(|| PathBuf::from("."));
	let to = if to.is_dir() {
		to.join(ops::export_file_name(project, asset)?)
	} else {
		to
	};
	ops::export_asset(project, asset, &to)?;
	println!("{asset} -> {}", to.display());
	Ok(())
}

fn list(project: &Project) -> ZResult<()> {
	let registry = AssetRegistry::scan(project)?;
	let mut assets: Vec<_> = registry.iter().collect();
	assets.sort_by(|(_, a), (_, b)| a.path.cmp(&b.path));
	for (id, entry) in assets {
		println!("{:<12} {id}  {}", format!("{:?}", entry.kind), entry.path);
	}
	println!("{} assets", registry.len());
	for problem in registry.problems() {
		match problem {
			AssetProblem::Duplicate { id, kept, ignored } => {
				println!("duplicate: {ignored} has the id {id} of {kept} (`zenith-cli new-id` fixes it)");
			}
			AssetProblem::Unreadable { path, error } => println!("unreadable: {path}: {error}"),
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use clap::CommandFactory;

	use super::*;

	#[test]
	fn the_command_line_is_well_formed() {
		Cli::command().debug_assert();
	}

	#[test]
	fn import_takes_a_folder_and_a_usage() {
		let cli = Cli::try_parse_from([
			"zenith-cli",
			"import",
			"sky.hdr",
			"--to",
			"Assets://Sky",
			"--usage",
			"hdr",
		]);
		assert!(
			matches!(
				cli.map(|cli| (cli.project, cli.command)),
				Ok((None, Command::Import { to, usage: Some(Usage::Hdr), .. })) if to == "Assets://Sky"
			),
			"parsed"
		);
		assert!(Cli::try_parse_from(["zenith-cli", "import"]).is_err(), "no source");
		assert!(
			Cli::try_parse_from(["zenith-cli", "import", "a.png", "--usage", "shiny"]).is_err(),
			"no such usage"
		);
	}

	#[test]
	fn the_project_can_be_given_before_or_after_the_command() {
		for args in [
			["zenith-cli", "-p", "Game", "assets"],
			["zenith-cli", "assets", "--project", "Game"],
		] {
			let cli = Cli::try_parse_from(args);
			assert!(
				matches!(cli.map(|cli| (cli.project, cli.command)), Ok((Some(p), Command::Assets)) if p == Path::new("Game")),
				"{args:?}"
			);
		}
	}

	#[test]
	fn the_project_is_found_from_a_folder_inside_it() -> ZResult<()> {
		let dir = tempfile::tempdir()?;
		let root = dir.path().join("Game");
		Project::create(&root)?;
		let inside = root.join("Assets/Models");
		std::fs::create_dir_all(&inside)?;
		assert_eq!(Project::find_folder(&inside).as_deref(), Some(root.as_path()));
		assert!(open_project(Some(root)).is_ok(), "given");
		Ok(())
	}
}
