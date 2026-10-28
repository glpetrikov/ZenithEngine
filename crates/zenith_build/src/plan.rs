//! Turns the rules into the final list of entries.

use std::{
	collections::BTreeMap,
	io::{Cursor, Read},
	sync::Arc,
	time::SystemTime,
};

use globset::{Glob, GlobBuilder, GlobMatcher};
use zenith_archive::{INDEX_FILE_NAME, validate_entry_set};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{EntryKind, Project, ZPath, layout};

use crate::builder::{BuildFormat, Rule, base_dirs};

/// Where an entry's bytes come from.
#[derive(Clone, Debug)]
pub enum Source {
	/// A file of the project folder, by project-relative path, with the size
	/// and modification time the walk saw (they let a rebuild skip hashing a
	/// file that has not changed).
	Project {
		rel: String,
		size: u64,
		modified: Option<SystemTime>,
	},
	/// A project file named through a scheme (`add_file`).
	Zpath(ZPath),
	/// Bytes supplied by the caller (`add_bytes`).
	Bytes(Arc<[u8]>),
}

/// The final entry list: build path to source, sorted by path.
pub struct Plan {
	pub entries: BTreeMap<String, Source>,
	/// Project-relative paths of the files the build reads, for the output
	/// overlap check.
	pub read_rels: Vec<String>,
}

pub type Matchers = Vec<Option<GlobMatcher>>;

/// A path the rules are evaluated for: a file of the walk, or only the
/// destination of an add step (`kind` is `None` then).
struct Candidate {
	kind: Option<EntryKind>,
	size: u64,
	modified: Option<SystemTime>,
}

/// Compiles the globs of `exclude`/`include` steps (`None` for the other
/// steps).
pub fn compile_rules(rules: &[Rule]) -> ZResult<Matchers> {
	rules
		.iter()
		.map(|rule| match rule {
			Rule::Exclude(glob) | Rule::Include(glob) => {
				// `*` must not cross `/`, as in .gitignore and most build
				// tools.
				let glob: Glob = GlobBuilder::new(glob)
					.literal_separator(true)
					.build()
					.map_err(|e| ZenithError::InvalidBuildOption(format!("glob {glob:?}: {e}")))?;
				Ok(Some(glob.compile_matcher()))
			}
			Rule::AddFile { .. } | Rule::AddBytes { .. } => Ok(None),
		})
		.collect()
}

fn top_component(path: &str) -> &str {
	path.split('/').next().unwrap_or(path)
}

/// Evaluates the rules for every candidate path; per path the last step that
/// applies wins.
pub fn make_plan(
	project: &Project,
	rules: &[Rule],
	matchers: &Matchers,
	format: BuildFormat,
	encrypted: bool,
) -> ZResult<Plan> {
	let base = base_dirs();
	let include_any = rules.iter().any(|r| matches!(r, Rule::Include(_)));
	// Without an include step nothing outside the base list can be needed, so
	// do not walk `Temp/`, `Trash/` or `.git/` at all. The repository's own
	// folder is never walked, include or not: it is the biggest tree in a
	// project and nothing in it belongs in a build.
	let descend = |dir: &str| {
		let top = top_component(dir);
		top != layout::DIR_GIT && (include_any || base.contains(&top))
	};
	let walked = project.walk_project(&descend)?;
	let manifest_file = layout::manifest_file_name(&project.manifest().name);

	// Candidates: every file the walk found, plus the destinations of add
	// steps.
	let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();
	for entry in walked {
		if entry.kind != EntryKind::Dir {
			candidates.insert(
				entry.path,
				Candidate {
					kind: Some(entry.kind),
					size: entry.size,
					modified: entry.modified,
				},
			);
		}
	}
	let mut dests: Vec<Option<String>> = Vec::with_capacity(rules.len());
	for rule in rules {
		let dest = match rule {
			Rule::AddFile { dest, src } => {
				if project.is_symlink(src)? {
					return Err(ZenithError::SymlinkInBuild(src.to_string()));
				}
				if project.metadata(src)?.kind != EntryKind::File {
					return Err(ZenithError::InvalidBuildOption(format!("{src} is not a file")));
				}
				Some(dest)
			}
			Rule::AddBytes { dest, .. } => Some(dest),
			Rule::Exclude(_) | Rule::Include(_) => None,
		};
		let dest = dest
			.map(|dest| {
				let rel = project.project_path(dest)?;
				if layout::is_trash_path(&rel) {
					return Err(ZenithError::InvalidBuildOption(format!("{dest} is in the trash")));
				}
				Ok(rel)
			})
			.transpose()?;
		if let Some(rel) = &dest {
			candidates.entry(rel.clone()).or_insert(Candidate {
				kind: None,
				size: 0,
				modified: None,
			});
		}
		dests.push(dest);
	}

	let mut entries = BTreeMap::new();
	let mut read_rels = Vec::new();
	for (path, candidate) in &candidates {
		let kind = candidate.kind;
		let mut included = kind.is_some() && (path == &manifest_file || base.contains(&top_component(path)));
		let mut replaced_by: Option<usize> = None;
		for (i, rule) in rules.iter().enumerate() {
			let hit = matchers
				.get(i)
				.and_then(Option::as_ref)
				.is_some_and(|m| m.is_match(path.as_str()));
			match rule {
				Rule::Exclude(_) if hit => {
					included = false;
					replaced_by = None;
				}
				// An include only adds files that exist in the project.
				Rule::Include(_) if hit && kind.is_some() => {
					included = true;
					replaced_by = None;
				}
				Rule::AddFile { .. } | Rule::AddBytes { .. }
					if dests.get(i).and_then(|d| d.as_deref()) == Some(path.as_str()) =>
				{
					included = true;
					replaced_by = Some(i);
				}
				_ => {}
			}
		}
		if !included {
			continue;
		}
		let source = match replaced_by.and_then(|i| rules.get(i)) {
			Some(Rule::AddFile { src, .. }) => {
				read_rels.push(project.project_path(src)?);
				Source::Zpath(src.clone())
			}
			Some(Rule::AddBytes { bytes, .. }) => Source::Bytes(Arc::from(bytes.as_slice())),
			_ => {
				// No silent skipping: a link in the entry list is an error.
				if kind == Some(EntryKind::Symlink) {
					return Err(ZenithError::SymlinkInBuild(path.clone()));
				}
				read_rels.push(path.clone());
				Source::Project {
					rel: path.clone(),
					size: candidate.size,
					modified: candidate.modified,
				}
			}
		};
		entries.insert(path.clone(), source);
	}

	let paths: Vec<&str> = entries.keys().map(String::as_str).collect();
	validate_entry_set(&paths)?;
	if format == BuildFormat::Fs && !encrypted && entries.contains_key(INDEX_FILE_NAME) {
		return Err(ZenithError::InvalidBuildOption(format!(
			"a file named {INDEX_FILE_NAME} in the project root would collide with the folder build's index"
		)));
	}
	Ok(Plan { entries, read_rels })
}

/// Opens the bytes of an entry.
pub fn open_source(project: &Project, source: &Source) -> ZResult<Box<dyn Read>> {
	Ok(match source {
		Source::Project { rel, .. } => Box::new(project.open_project_file(rel)?),
		Source::Zpath(path) => Box::new(project.open_read(path)?),
		Source::Bytes(bytes) => Box::new(Cursor::new(bytes.clone())),
	})
}
