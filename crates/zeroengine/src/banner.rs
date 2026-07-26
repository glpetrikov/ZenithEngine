use std::{
	io::{self, Write},
	path::{Component, Path, PathBuf},
};

use owo_colors::OwoColorize;
use terminal_size::{Width, terminal_size};

const MIN_BANNER_WIDTH: usize = 40;
const BANNER_TERMINAL_MARGIN: usize = 4;
const BANNER_HORIZONTAL_PADDING: usize = 6;
const DIRECTORY_LABEL: &str = "directory:";

pub fn render_banner() {
	let max_width = terminal_size().map_or(usize::MAX, |(Width(width), _)| {
		usize::from(width).saturating_sub(BANNER_TERMINAL_MARGIN)
	});
	let max_content_width = max_width.saturating_sub(BANNER_HORIZONTAL_PADDING);

	let title = format!("ZeroEngine (v{})", env!("CARGO_PKG_VERSION"));
	let directory = directory_line(max_content_width);

	let directory_line_width = directory.raw_width();
	let desired_width = title
		.chars()
		.count()
		.max(directory_line_width)
		.saturating_add(BANNER_HORIZONTAL_PADDING)
		.max(MIN_BANNER_WIDTH);
	let box_width = desired_width.min(max_width);
	let content_width = box_width.saturating_sub(BANNER_HORIZONTAL_PADDING);
	let border = "─".repeat(box_width.saturating_sub(2));

	let output = format!(
		"{}\n{}\n{}\n{}\n{}\n",
		format!("╭{border}╮").bright_black(),
		banner_line_segments(&[(title.clone(), title.white().bold().to_string())], content_width),
		banner_line("", content_width).bright_black(),
		banner_line_segments(&directory.segments(), content_width),
		format!("╰{border}╯").bright_black()
	);

	let mut stdout = io::stdout().lock();
	if let Err(error) = stdout.write_all(output.as_bytes()).and_then(|()| stdout.flush()) {
		eprintln!("Failed to write banner: {error}");
	}
}

fn banner_line(text: &str, content_width: usize) -> String { format!("│  {text:<content_width$}  │") }

struct DirectoryLine {
	label: Option<&'static str>,
	path: String,
}

impl DirectoryLine {
	fn raw_width(&self) -> usize { self.label.map_or(0, |label| label.chars().count() + 1) + self.path.chars().count() }

	fn segments(&self) -> Vec<(String, String)> {
		self.label.map_or_else(
			|| vec![(self.path.clone(), directory_path_segment(&self.path))],
			|label| {
				let path_raw = format!(" {}", self.path);
				let path_styled = format!("{}{}", " ".default_color(), directory_path_segment(&self.path));
				vec![
					(label.to_string(), label.bright_black().to_string()),
					(path_raw, path_styled),
				]
			},
		)
	}
}

fn directory_line(max_content_width: usize) -> DirectoryLine {
	let full_path = compact_current_dir(usize::MAX);
	let labeled_width = directory_label_width_with_space() + full_path.chars().count();

	if labeled_width <= max_content_width {
		return DirectoryLine {
			label: Some(DIRECTORY_LABEL),
			path: full_path,
		};
	}

	DirectoryLine {
		label: None,
		path: compact_current_dir(max_content_width),
	}
}

fn directory_label_width_with_space() -> usize { DIRECTORY_LABEL.chars().count() + 1 }

fn directory_path_segment(path: &str) -> String { path.bright_white().to_string() }

fn banner_line_segments(segments: &[(String, String)], content_width: usize) -> String {
	let raw_width = segments
		.iter()
		.map(|(raw_text, _)| raw_text.chars().count())
		.sum::<usize>();
	let padding = content_width.saturating_sub(raw_width);
	let content = segments
		.iter()
		.map(|(_, styled_text)| styled_text.as_str())
		.collect::<String>();

	format!(
		"{}{}{}",
		"│  ".bright_black(),
		content,
		format!("{}  │", " ".repeat(padding)).bright_black()
	)
}

fn compact_current_dir(max_width: usize) -> String {
	let Ok(current_dir) = std::env::current_dir() else {
		return "<unknown>".to_string();
	};

	if let Some(home_dir) = home_dir()
		&& current_dir.starts_with(&home_dir)
	{
		let relative = current_dir.strip_prefix(&home_dir).unwrap_or(&current_dir);
		let (_, _, parts) = path_parts(relative);
		return compact_home_path(&parts, max_width);
	}

	let (drive, absolute, parts) = path_parts(&current_dir);
	compact_external_path(drive.as_deref(), absolute, &parts, max_width)
}

fn compact_home_path(parts: &[String], max_width: usize) -> String {
	if parts.is_empty() {
		return "~".to_string();
	}

	let full = format!("~/{}", parts.join("/"));
	if full.chars().count() <= max_width {
		return full;
	}

	compact_with_ellipsis("~/", parts, max_width)
}

fn compact_external_path(drive: Option<&str>, absolute: bool, parts: &[String], max_width: usize) -> String {
	let prefix = match (drive, absolute) {
		(Some(drive), _) => format!("{drive}/"),
		(None, true) => "/".to_string(),
		(None, false) => String::new(),
	};
	let full = format!("{prefix}{}", parts.join("/"));

	if full.chars().count() <= max_width {
		return full;
	}

	compact_with_ellipsis(&prefix, parts, max_width)
}

fn compact_with_ellipsis(prefix: &str, parts: &[String], max_width: usize) -> String {
	let min_components = parts.len().min(2);

	for component_count in (min_components..=parts.len()).rev() {
		let start = parts.len() - component_count;
		let candidate = format!("{prefix}.../{}", parts[start..].join("/"));
		if candidate.chars().count() <= max_width {
			return candidate;
		}
	}

	let start = parts.len().saturating_sub(min_components);
	format!("{prefix}.../{}", parts[start..].join("/"))
}

fn path_parts(path: &Path) -> (Option<String>, bool, Vec<String>) {
	let mut drive = None;
	let mut absolute = false;
	let mut parts = Vec::new();

	for component in path.components() {
		match component {
			Component::Prefix(prefix) => {
				drive = drive_from_prefix(&prefix.as_os_str().to_string_lossy());
			}
			Component::RootDir => {
				absolute = true;
			}
			Component::Normal(part) => {
				parts.push(part.to_string_lossy().replace('\\', "/"));
			}
			Component::CurDir | Component::ParentDir => {}
		}
	}

	(drive, absolute, parts)
}

fn drive_from_prefix(prefix: &str) -> Option<String> {
	let normalized = prefix.replace('\\', "/");
	let bytes = normalized.as_bytes();

	for window in bytes.windows(2) {
		if window[0].is_ascii_alphabetic() && window[1] == b':' {
			let drive = char::from(window[0]).to_ascii_uppercase();
			return Some(format!("{drive}:"));
		}
	}

	Some(normalized.trim_end_matches('/').to_string()).filter(|drive| !drive.is_empty())
}

fn home_dir() -> Option<PathBuf> {
	if cfg!(windows) {
		std::env::var_os("USERPROFILE").map(PathBuf::from).or_else(|| {
			let drive = std::env::var_os("HOMEDRIVE")?;
			let path = std::env::var_os("HOMEPATH")?;
			let mut home = PathBuf::from(drive);
			home.push(path);
			Some(home)
		})
	} else {
		std::env::var_os("HOME").map(PathBuf::from)
	}
}
