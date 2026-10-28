//! The Zenith-managed block of a project's `.gitignore`.
//!
//! Entries Zenith adds live between `# zenith:begin` and `# zenith:end`.
//! Everything outside the block is never touched. Functions here are pure
//! text transformations: they return the new text, or `None` when nothing has
//! to change, and keep the file's line endings and trailing newline.

use zenith_error::{ZResult, ZenithError};

pub const BEGIN: &str = "# zenith:begin";
pub const END: &str = "# zenith:end";

fn corrupt(why: &str) -> ZenithError {
	ZenithError::InvalidGitignore(why.to_string())
}

#[derive(Clone, Debug)]
struct Line {
	text: String,
	/// `"\n"`, `"\r\n"`, or empty for an unterminated last line.
	eol: String,
}

#[derive(Debug)]
struct Doc {
	lines: Vec<Line>,
}

impl Doc {
	fn parse(text: &str) -> Self {
		let lines = text
			.split_inclusive('\n')
			.map(|segment| {
				let eol = if segment.ends_with("\r\n") {
					"\r\n"
				} else if segment.ends_with('\n') {
					"\n"
				} else {
					""
				};
				Line {
					text: segment.strip_suffix(eol).unwrap_or(segment).to_string(),
					eol: eol.to_string(),
				}
			})
			.collect();
		Self { lines }
	}

	fn render(&self) -> String {
		self.lines.iter().fold(String::new(), |mut out, l| {
			out.push_str(&l.text);
			out.push_str(&l.eol);
			out
		})
	}

	/// Line ending used for lines we add: the file's own, `\n` for a new file.
	fn eol(&self) -> String {
		self.lines
			.iter()
			.find(|l| !l.eol.is_empty())
			.map_or_else(|| "\n".to_string(), |l| l.eol.clone())
	}

	fn ends_without_newline(&self) -> bool {
		self.lines.last().is_some_and(|l| l.eol.is_empty())
	}

	/// Finds the managed block. Anything but zero or one well-formed block is
	/// treated as damage: guessing which markers are real could eat user lines.
	fn block(&self) -> ZResult<Option<(usize, usize)>> {
		let mut begin = None;
		let mut found = None;
		for (i, line) in self.lines.iter().enumerate() {
			let text = line.text.trim_end();
			if text == BEGIN {
				if begin.is_some() {
					return Err(corrupt("`# zenith:begin` appears twice without `# zenith:end` between"));
				}
				if found.is_some() {
					return Err(corrupt("more than one Zenith block"));
				}
				begin = Some(i);
			} else if text == END {
				let Some(start) = begin.take() else {
					return Err(corrupt("`# zenith:end` without `# zenith:begin`"));
				};
				found = Some((start, i));
			}
		}
		if begin.is_some() {
			return Err(corrupt("`# zenith:begin` without `# zenith:end`"));
		}
		Ok(found)
	}
}

/// `/Build/`, `Build`, ` build/ ` all mean the folder `Build`.
fn normalize(entry: &str) -> &str {
	entry.trim().trim_matches('/')
}

fn is_entry(line: &str) -> bool {
	let t = line.trim();
	!t.is_empty() && !t.starts_with('#')
}

/// Patterns Zenith did not write; they are never removed automatically.
fn is_plain_path(entry: &str) -> bool {
	!entry.is_empty() && !entry.starts_with('!') && !entry.contains(['*', '?', '[', '\\'])
}

/// Folders listed in the managed block (without slashes). Fails on a damaged
/// block.
///
/// # Errors
/// `InvalidGitignore` if the block is damaged.
pub fn managed_entries(text: &str) -> ZResult<Vec<String>> {
	let doc = Doc::parse(text);
	let Some((begin, end)) = doc.block()? else {
		return Ok(Vec::new());
	};
	Ok(doc
		.lines
		.iter()
		.take(end)
		.skip(begin.saturating_add(1))
		.filter(|l| is_entry(&l.text) && is_plain_path(normalize(&l.text)))
		.map(|l| normalize(&l.text).to_string())
		.collect())
}

/// Whether the file already ignores `dir` (the folder itself or a parent),
/// through the managed block or any line outside it.
#[must_use]
pub fn ignores(text: &str, dir: &str) -> bool {
	let doc = Doc::parse(text);
	let mut ancestor = dir;
	loop {
		if doc
			.lines
			.iter()
			.any(|l| is_entry(&l.text) && normalize(&l.text) == ancestor)
		{
			return true;
		}
		match ancestor.rsplit_once('/') {
			Some((parent, _)) => ancestor = parent,
			None => return false,
		}
	}
}

/// Adds `/<dir>/` to the managed block, creating the block if needed.
/// Returns `None` if an equivalent line already exists inside or outside the
/// block.
///
/// # Errors
/// `InvalidGitignore` if the block is damaged; nothing is changed then.
pub fn add_entry(text: &str, dir: &str) -> ZResult<Option<String>> {
	let mut doc = Doc::parse(text);
	let block = doc.block()?;
	if ignores_exact(&doc, dir) {
		return Ok(None);
	}
	let eol = doc.eol();
	let entry = Line {
		text: format!("/{dir}/"),
		eol: eol.clone(),
	};
	if let Some((_, end)) = block {
		doc.lines.insert(end, entry);
	} else {
		let unterminated = doc.ends_without_newline();
		if let Some(last) = doc.lines.last_mut().filter(|l| l.eol.is_empty()) {
			last.eol.clone_from(&eol);
		}
		doc.lines.push(Line {
			text: BEGIN.to_string(),
			eol: eol.clone(),
		});
		doc.lines.push(entry);
		doc.lines.push(Line {
			text: END.to_string(),
			eol: if unterminated { String::new() } else { eol },
		});
	}
	Ok(Some(doc.render()))
}

fn ignores_exact(doc: &Doc, dir: &str) -> bool {
	doc.lines.iter().any(|l| is_entry(&l.text) && normalize(&l.text) == dir)
}

/// Removes block entries whose folder no longer exists, and the whole block
/// once it holds no entries. Returns `None` if nothing changed.
///
/// # Errors
/// `InvalidGitignore` if the block is damaged; nothing is changed then.
pub fn remove_missing(text: &str, exists: impl Fn(&str) -> bool) -> ZResult<Option<String>> {
	let mut doc = Doc::parse(text);
	let Some((begin, end)) = doc.block()? else {
		return Ok(None);
	};

	let doomed: Vec<usize> = (begin.saturating_add(1)..end)
		.filter(|i| {
			doc.lines.get(*i).is_some_and(|l| {
				let path = normalize(&l.text);
				is_entry(&l.text) && is_plain_path(path) && !exists(path)
			})
		})
		.collect();
	if doomed.is_empty() {
		return Ok(None);
	}
	let remaining = (begin.saturating_add(1)..end)
		.filter(|i| !doomed.contains(i) && doc.lines.get(*i).is_some_and(|l| is_entry(&l.text)))
		.count();

	let unterminated = doc.ends_without_newline();
	let remove: Vec<usize> = if remaining == 0 {
		(begin..=end).collect()
	} else {
		doomed
	};
	let mut index = 0;
	doc.lines.retain(|_| {
		let keep = !remove.contains(&index);
		index += 1;
		keep
	});
	if unterminated && let Some(last) = doc.lines.last_mut() {
		last.eol.clear();
	}
	Ok(Some(doc.render()))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn block_of(text: &str) -> Vec<String> {
		managed_entries(text).unwrap_or_default()
	}

	#[test]
	fn creates_the_block_at_the_end() -> ZResult<()> {
		let text = "Temp/\nTrash/\n";
		let out = add_entry(text, "Build")?.ok_or_else(|| corrupt("expected a change"))?;
		assert_eq!(
			out, "Temp/\nTrash/\n# zenith:begin\n/Build/\n# zenith:end\n",
			"block appended, lines outside untouched"
		);
		assert_eq!(block_of(&out), vec!["Build"], "entry is readable");
		let again = add_entry(&out, "Build")?;
		assert!(again.is_none(), "the same entry is not added twice");
		let more = add_entry(&out, "Out/Dist")?.ok_or_else(|| corrupt("expected a change"))?;
		assert_eq!(
			more, "Temp/\nTrash/\n# zenith:begin\n/Build/\n/Out/Dist/\n# zenith:end\n",
			"second entry goes inside the block"
		);
		Ok(())
	}

	#[test]
	fn keeps_line_endings_and_missing_final_newline() -> ZResult<()> {
		let crlf = add_entry("Temp/\r\nTrash/\r\n", "Build")?.unwrap_or_default();
		assert_eq!(
			crlf, "Temp/\r\nTrash/\r\n# zenith:begin\r\n/Build/\r\n# zenith:end\r\n",
			"CRLF file stays CRLF"
		);

		let bare = add_entry("Temp/\nTrash/", "Build")?.unwrap_or_default();
		assert_eq!(
			bare, "Temp/\nTrash/\n# zenith:begin\n/Build/\n# zenith:end",
			"no final newline stays absent"
		);

		let fresh = add_entry("", "Build")?.unwrap_or_default();
		assert_eq!(
			fresh, "# zenith:begin\n/Build/\n# zenith:end\n",
			"empty file gets a terminated block"
		);
		Ok(())
	}

	#[test]
	fn entry_already_present_outside_the_block_is_not_duplicated() -> ZResult<()> {
		for existing in ["Build/\n", "/Build/\n", "/Build\n", "Build\n"] {
			assert!(
				add_entry(existing, "Build")?.is_none(),
				"{existing:?} already ignores Build"
			);
		}
		assert!(add_entry("# Build/\n", "Build")?.is_some(), "a comment is not an entry");
		assert!(ignores("Out/\n", "Out/Dist"), "a parent folder counts");
		assert!(!ignores("Out/\n", "Outer"), "a name prefix is not a parent");
		Ok(())
	}

	#[test]
	fn removes_missing_entries_and_the_empty_block() -> ZResult<()> {
		let text = "Temp/\n# zenith:begin\n/Build/\n/Out/\n# zenith:end\nkeep-me\n";
		let one = remove_missing(text, |p| p == "Out")?.unwrap_or_default();
		assert_eq!(
			one, "Temp/\n# zenith:begin\n/Out/\n# zenith:end\nkeep-me\n",
			"only the missing folder goes"
		);
		let none = remove_missing(text, |_| true)?;
		assert!(none.is_none(), "nothing missing, nothing changes");
		let all = remove_missing(text, |_| false)?.unwrap_or_default();
		assert_eq!(all, "Temp/\nkeep-me\n", "an empty block disappears entirely");

		let tail = "Temp/\n# zenith:begin\n/Build/\n# zenith:end";
		assert_eq!(
			remove_missing(tail, |_| false)?.unwrap_or_default(),
			"Temp/",
			"unterminated file stays unterminated"
		);
		Ok(())
	}

	#[test]
	fn hand_written_patterns_in_the_block_are_left_alone() -> ZResult<()> {
		let text = "# zenith:begin\n*.tmp\n/Gone/\n# zenith:end\n";
		let out = remove_missing(text, |_| false)?.unwrap_or_default();
		assert_eq!(
			out, "# zenith:begin\n*.tmp\n# zenith:end\n",
			"globs are never removed automatically"
		);
		Ok(())
	}

	#[test]
	fn damaged_blocks_are_errors_and_change_nothing() {
		for text in [
			"# zenith:begin\n/Build/\n",
			"/Build/\n# zenith:end\n",
			"# zenith:begin\n# zenith:begin\n# zenith:end\n",
			"# zenith:begin\n# zenith:end\n# zenith:begin\n# zenith:end\n",
			"# zenith:end\n# zenith:begin\n",
		] {
			assert!(add_entry(text, "X").is_err(), "{text:?} must be rejected by add_entry");
			assert!(
				remove_missing(text, |_| false).is_err(),
				"{text:?} must be rejected by remove_missing"
			);
			assert!(
				managed_entries(text).is_err(),
				"{text:?} must be rejected by managed_entries"
			);
		}
	}

	#[test]
	fn no_block_means_nothing_to_sync() -> ZResult<()> {
		assert!(remove_missing("Temp/\n", |_| false)?.is_none(), "no block");
		assert!(remove_missing("", |_| false)?.is_none(), "empty file");
		Ok(())
	}
}
