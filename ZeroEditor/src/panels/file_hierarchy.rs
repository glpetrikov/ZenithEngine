use std::{
	collections::HashSet,
	fs,
	path::{Component, Path, PathBuf},
	time::Instant,
};

use egui::{CornerRadius, Ui};
use serde::{Deserialize, Serialize};
use ze_project::PROJECT_FILE_NAME;

use super::{
	EditorPanelContext, EditorSelection, Panel,
	texture_sheet_create_dialog::{TextureSheetCreateDialog, TextureSheetCreateOutcome},
};

const TILE_SIZE: egui::Vec2 = egui::vec2(100.0, 120.0);
const TILE_SPACING: f32 = 8.0;
const LEFT_PANEL_WIDTH: f32 = 200.0;

#[derive(Debug, Clone)]
struct ClipboardEntry {
	source: PathBuf,
	cut: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingCreateKind {
	Folder,
	CSharpScript,
	CSharpComponent,
	Scene,
	Shader,
	AnimationClip,
}

impl PendingCreateKind {
	const fn title(self) -> &'static str {
		match self {
			Self::Folder => "New Folder",
			Self::CSharpScript => "C# Script",
			Self::CSharpComponent => "C# Component",
			Self::Scene => "New Scene",
			Self::Shader => "New Shader",
			Self::AnimationClip => "New Animation Clip",
		}
	}

	const fn prompt(self) -> &'static str {
		match self {
			Self::Folder => "Folder name",
			Self::CSharpScript => "Script name",
			Self::CSharpComponent => "Component name",
			Self::Scene => "Scene name",
			Self::Shader => "Shader name",
			Self::AnimationClip => "Animation clip name",
		}
	}
}

#[derive(Debug)]
struct PendingCreate {
	kind: PendingCreateKind,
	name: String,
	error: Option<String>,
	/// Only set for `PendingCreateKind::AnimationClip`: the
	/// `.texturesheet.json` that was right-clicked to trigger this prompt, and
	/// that the new clip will reference.
	source_sheet: Option<PathBuf>,
}

impl PendingCreate {
	const fn new(kind: PendingCreateKind) -> Self {
		Self {
			kind,
			name: String::new(),
			error: None,
			source_sheet: None,
		}
	}

	fn for_animation_clip(source_sheet: PathBuf) -> Self {
		Self {
			source_sheet: Some(source_sheet),
			..Self::new(PendingCreateKind::AnimationClip)
		}
	}
}

#[derive(Debug, Serialize, Deserialize)]
struct FavoritesData {
	favorites: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TreeExpandedData {
	expanded: Vec<String>,
}

#[derive(Debug)]
pub struct FileExplorer {
	project_root: PathBuf,
	current_path: PathBuf,
	back_history: Vec<PathBuf>,
	forward_history: Vec<PathBuf>,
	pending_create: Option<PendingCreate>,
	pending_texture_sheet_create: Option<TextureSheetCreateDialog>,
	status_flash: Option<(String, Instant)>,

	favorites: Vec<PathBuf>,
	favorites_dirty: bool,
	tree_expanded: HashSet<PathBuf>,
	tree_expanded_dirty: bool,
	selected_entry: Option<String>,
	clipboard: Option<ClipboardEntry>,
	renaming: Option<String>,
	grid_focused: bool,
}

impl Default for FileExplorer {
	fn default() -> Self { Self::new(PathBuf::from(".")) }
}

impl FileExplorer {
	pub fn new(root_path: PathBuf) -> Self {
		let root_path = Self::normalize_directory_path(root_path);
		let project_root = Self::find_project_root(&root_path).unwrap_or_else(|| root_path.clone());

		let mut explorer = Self {
			project_root,
			current_path: root_path,
			back_history: Vec::new(),
			forward_history: Vec::new(),
			pending_create: None,
			pending_texture_sheet_create: None,
			status_flash: None,
			favorites: Vec::new(),
			favorites_dirty: false,
			tree_expanded: HashSet::new(),
			tree_expanded_dirty: false,
			selected_entry: None,
			clipboard: None,
			renaming: None,
			grid_focused: false,
		};

		explorer.load_favorites();
		explorer.load_tree_expanded();
		explorer
	}

	// ---------------------------------------------------------------------------
	// Favorites persistence
	// ---------------------------------------------------------------------------

	fn favorites_path(&self) -> PathBuf { self.project_root.join(FAVORITES_DIR) }

	fn load_favorites(&mut self) {
		let path = self.favorites_path();
		if !path.is_file() {
			return;
		}

		let Ok(content) = fs::read_to_string(&path) else {
			ze_log::warn!("Failed to read favorites file: {}", path.display());
			return;
		};
		let Ok(data) = toml::from_str::<FavoritesData>(&content) else {
			return;
		};

		self.favorites = data
			.favorites
			.into_iter()
			.map(|rel| Self::normalize_directory_path(self.project_root.join(rel)))
			.filter(|p| p.is_dir())
			.collect();
	}

	fn save_favorites(&self) {
		if !self.favorites_dirty {
			return;
		}

		let rel_paths: Vec<String> = self
			.favorites
			.iter()
			.filter_map(|abs| abs.strip_prefix(&self.project_root).ok())
			.map(|rel| rel.display().to_string())
			.collect();

		let data = FavoritesData { favorites: rel_paths };

		let Ok(content) = toml::to_string(&data) else {
			ze_log::warn!("Failed to serialize favorites");
			return;
		};

		let _ = fs::create_dir_all(self.project_root.join(".zeroeditor"));
		if let Err(error) = fs::write(self.favorites_path(), content) {
			ze_log::warn!("Failed to write favorites file: {error}");
		}
	}

	const fn mark_favorites_dirty(&mut self) { self.favorites_dirty = true; }

	// ---------------------------------------------------------------------------
	// Tree expansion state persistence
	// ---------------------------------------------------------------------------

	fn tree_expanded_path(&self) -> PathBuf { self.project_root.join(TREE_EXPANDED_DIR) }

	fn load_tree_expanded(&mut self) {
		let path = self.tree_expanded_path();
		if !path.is_file() {
			self.tree_expanded = Self::collect_all_subdirs(&self.project_root);
			self.tree_expanded_dirty = true;
			return;
		}

		let Ok(content) = fs::read_to_string(&path) else {
			return;
		};
		let Ok(data) = toml::from_str::<TreeExpandedData>(&content) else {
			return;
		};

		self.tree_expanded = data
			.expanded
			.into_iter()
			.map(|rel| self.project_root.join(rel))
			.filter(|p| p.exists())
			.collect();
	}

	fn save_tree_expanded(&self) {
		if !self.tree_expanded_dirty {
			return;
		}

		let rel_paths: Vec<String> = self
			.tree_expanded
			.iter()
			.filter_map(|abs| abs.strip_prefix(&self.project_root).ok())
			.map(|rel| rel.display().to_string())
			.collect();

		let data = TreeExpandedData { expanded: rel_paths };

		let Ok(content) = toml::to_string(&data) else {
			return;
		};

		let _ = fs::create_dir_all(self.project_root.join(".zeroeditor"));
		if let Err(error) = fs::write(self.tree_expanded_path(), content) {
			ze_log::warn!("Failed to write tree expanded file: {error}");
		}
	}

	const fn mark_tree_expanded_dirty(&mut self) { self.tree_expanded_dirty = true; }

	fn collect_all_subdirs(root: &Path) -> HashSet<PathBuf> {
		let mut result = HashSet::new();
		let Ok(entries) = fs::read_dir(root) else {
			return result;
		};
		for entry in entries.filter_map(Result::ok) {
			let path = entry.path();
			if path.is_dir() {
				result.insert(path.clone());
				result.extend(Self::collect_all_subdirs(&path));
			}
		}
		result
	}

	// ---------------------------------------------------------------------------
	// Navigation
	// ---------------------------------------------------------------------------

	fn go_back(&mut self) {
		let Some(path) = self.pop_valid_back_history() else {
			return;
		};

		self.forward_history.push(self.current_path.clone());

		self.current_path = path;
	}

	fn go_forward(&mut self) {
		let Some(path) = self.pop_valid_forward_history() else {
			return;
		};

		self.back_history.push(self.current_path.clone());

		self.current_path = path;
	}

	fn open_directory(&mut self, path: PathBuf) {
		let path = Self::normalize_directory_path(path);
		if !self.is_within_project_root(&path) {
			return;
		}
		if !path.is_dir() {
			return;
		}
		if self.current_path == path {
			return;
		}

		self.back_history.push(self.current_path.clone());
		self.forward_history.clear();

		self.current_path = path;
		self.selected_entry = None;
	}

	fn normalize_directory_path(path: PathBuf) -> PathBuf {
		let path = if path.as_os_str().is_empty() {
			std::env::current_dir().unwrap_or_else(|error| {
				ze_log::warn!("Failed to normalize empty File Explorer path: {error}");
				PathBuf::from(".")
			})
		} else if path.is_relative() {
			match std::env::current_dir() {
				Ok(current_dir) => current_dir.join(path),
				Err(_) => path,
			}
		} else {
			path
		};

		path.canonicalize().unwrap_or(path)
	}

	fn is_within_project_root(&self, path: &Path) -> bool { path.starts_with(&self.project_root) }

	fn find_project_root(path: &Path) -> Option<PathBuf> {
		for ancestor in path.ancestors() {
			if ancestor.join(PROJECT_FILE_NAME).is_file() {
				return Some(Self::normalize_directory_path(ancestor.to_path_buf()));
			}
		}

		None
	}

	#[expect(dead_code, reason = "available for future navigation UI")]
	fn valid_parent(path: &Path) -> Option<PathBuf> {
		if path.as_os_str().is_empty() {
			return None;
		}

		let parent = path.parent()?;
		(!parent.as_os_str().is_empty()).then(|| parent.to_path_buf())
	}

	fn ensure_current_path_is_valid(&mut self) {
		if self.current_path.as_os_str().is_empty() || !self.is_within_project_root(&self.current_path) {
			self.current_path = self.project_root.clone();
		}
	}

	fn pop_valid_back_history(&mut self) -> Option<PathBuf> {
		while let Some(path) = self.back_history.pop() {
			let path = Self::normalize_directory_path(path);
			if self.is_within_project_root(&path) {
				return Some(path);
			}
		}

		None
	}

	fn pop_valid_forward_history(&mut self) -> Option<PathBuf> {
		while let Some(path) = self.forward_history.pop() {
			let path = Self::normalize_directory_path(path);
			if self.is_within_project_root(&path) {
				return Some(path);
			}
		}

		None
	}

	// ---------------------------------------------------------------------------
	// .zeignore
	// ---------------------------------------------------------------------------

	fn load_ignore_patterns(&self) -> Vec<(PathBuf, String)> { self.load_ignore_patterns_for(&self.current_path) }

	fn load_ignore_patterns_for(&self, dir: &Path) -> Vec<(PathBuf, String)> {
		let mut patterns = Vec::new();
		let mut current = Some(dir);

		while let Some(dir) = current {
			if !self.is_within_project_root(dir) {
				break;
			}
			let ignore_file = dir.join(".zeignore");
			if ignore_file.is_file()
				&& let Ok(content) = std::fs::read_to_string(&ignore_file)
			{
				for line in content.lines() {
					let line = line.trim();
					if line.is_empty() || line.starts_with('#') {
						continue;
					}
					patterns.push((dir.to_path_buf(), line.to_string()));
				}
			}
			current = dir.parent();
		}

		patterns
	}

	fn matches_ignore_pattern(entry_path: &Path, patterns: &[(PathBuf, String)]) -> bool {
		for (ignore_dir, pattern) in patterns {
			if let Ok(relative) = entry_path.strip_prefix(ignore_dir)
				&& Self::match_pattern(relative, pattern)
			{
				return true;
			}
		}
		false
	}

	fn match_pattern(relative: &Path, pattern: &str) -> bool {
		let pattern = pattern.trim();
		let anchored = pattern.starts_with('/');
		let effective = pattern.trim_start_matches('/');
		let effective = effective.strip_suffix('/').unwrap_or(effective);

		if !effective.contains('/') {
			let relative_name = relative.file_name().and_then(std::ffi::OsStr::to_str).unwrap_or("");
			return Self::match_flat_name(relative_name, effective);
		}

		let relative_str = relative.to_str().unwrap_or("");
		if anchored {
			relative_str == effective || relative_str.starts_with(&(effective.to_owned() + "/"))
		} else {
			relative_str == effective
				|| relative_str.starts_with(&(effective.to_owned() + "/"))
				|| relative_str.ends_with(&("/".to_owned() + effective))
				|| relative_str.contains(&("/".to_owned() + effective + "/"))
		}
	}

	fn match_flat_name(name: &str, pattern: &str) -> bool {
		if pattern == name {
			return true;
		}
		if pattern == "*" {
			return true;
		}
		if let Some(prefix) = pattern.strip_suffix('*')
			&& name.starts_with(prefix)
		{
			return true;
		}
		if let Some(suffix) = pattern.strip_prefix('*')
			&& name.ends_with(suffix)
		{
			return true;
		}
		if pattern.contains('*') {
			let parts: Vec<&str> = pattern.split('*').collect();
			if parts.len() == 2 {
				let prefix = parts[0];
				let suffix = parts[1];
				if name.starts_with(prefix) && name.ends_with(suffix) && name.len() >= prefix.len() + suffix.len() {
					return true;
				}
			}
		}
		false
	}

	// ---------------------------------------------------------------------------
	// Entry reading
	// ---------------------------------------------------------------------------

	fn read_subdirectories(path: &Path) -> Vec<PathBuf> {
		let Ok(entries) = std::fs::read_dir(path) else {
			return Vec::new();
		};

		let mut dirs: Vec<PathBuf> = entries
			.filter_map(Result::ok)
			.filter(|e| e.path().is_dir())
			.map(|e| e.path())
			.collect();

		dirs.sort();
		dirs
	}

	// ---------------------------------------------------------------------------
	// Icons
	// ---------------------------------------------------------------------------

	fn entry_icon(path: &Path, is_directory: bool) -> &'static str {
		if is_directory {
			"\u{F024B}"
		} else {
			match path.extension().and_then(|s| s.to_str()) {
				Some("cs") => "\u{F031B}",
				Some("toml") => "\u{F016A}",
				Some("wesl") => "\u{F0E60}",
				Some("zero") => "\u{F0381}",
				_ => "\u{F0214}",
			}
		}
	}

	const fn entry_icon_color(is_directory: bool) -> egui::Color32 {
		if is_directory {
			egui::Color32::from_rgb(95, 145, 220)
		} else {
			egui::Color32::from_rgb(120, 124, 132)
		}
	}

	const fn entry_text_color(is_directory: bool) -> egui::Color32 {
		if is_directory {
			egui::Color32::from_rgb(185, 215, 255)
		} else {
			egui::Color32::from_rgb(214, 214, 218)
		}
	}

	// ---------------------------------------------------------------------------
	// File operations
	// ---------------------------------------------------------------------------

	fn is_protected_path(&self, path: &Path) -> bool {
		let assets_dir = self.project_root.join("assets");
		path.strip_prefix(&assets_dir).is_ok_and(|relative| {
			let first_component = relative.components().next();
			matches!(first_component, Some(std::path::Component::Normal(_)))
		})
	}

	fn inspect_file(&self, context: &mut EditorPanelContext<'_>, path: &Path) {
		let Some(path) = Self::canonical_existing_path(path) else {
			return;
		};
		if !self.is_within_project_root(&path) {
			return;
		}

		*context.selection = Some(EditorSelection::Asset(path));
	}

	fn canonical_existing_path(path: &Path) -> Option<PathBuf> {
		match path.canonicalize() {
			Ok(path) => Some(path),
			Err(error) => {
				ze_log::warn!("File Explorer cannot canonicalize {}: {error}", path.display());
				None
			}
		}
	}

	fn set_status(&mut self, message: impl Into<String>) {
		let message = message.into();
		ze_log::debug!("{message}");
		self.status_flash = Some((message, Instant::now()));
	}

	// ---------------------------------------------------------------------------
	// Validation
	// ---------------------------------------------------------------------------

	fn validate_new_folder_name(name: &str) -> Result<&str, String> {
		let name = name.trim();
		if name.is_empty() {
			return Err("Folder name cannot be empty".to_string());
		}
		if name.contains('/') || name.contains('\\') {
			return Err("Folder name cannot contain path separators".to_string());
		}

		let mut components = Path::new(name).components();
		if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
			return Err("Folder name is not a plain folder name".to_string());
		}

		Ok(name)
	}

	fn validate_new_file_name(name: &str) -> Result<&str, String> {
		let name = name.trim();
		if name.is_empty() {
			return Err("File name cannot be empty".to_string());
		}
		if name.contains('/') || name.contains('\\') {
			return Err("File name cannot contain path separators".to_string());
		}

		let mut components = Path::new(name).components();
		if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
			return Err("File name is not a plain file name".to_string());
		}

		Ok(name)
	}

	fn new_folder_path(&mut self, name: &str) -> Result<PathBuf, String> {
		self.ensure_current_path_is_valid();
		let name = Self::validate_new_folder_name(name)?;
		let path = self.current_path.join(name);
		if !self.is_within_project_root(&path) {
			return Err("Cannot create folder outside project root".to_string());
		}
		if path.exists() {
			return Err(format!("Folder '{name}' already exists"));
		}

		Ok(path)
	}

	fn new_file_path(&mut self, name_with_ext: &str) -> Result<PathBuf, String> {
		self.ensure_current_path_is_valid();
		let name = Self::validate_new_file_name(name_with_ext)?;
		let path = self.current_path.join(name);
		if !self.is_within_project_root(&path) {
			return Err("Cannot create file outside project root".to_string());
		}
		if path.exists() {
			return Err(format!("File '{name}' already exists"));
		}

		Ok(path)
	}

	// ---------------------------------------------------------------------------
	// Create operations
	// ---------------------------------------------------------------------------

	fn create_folder(&mut self, name: &str) -> Result<(), String> {
		let path = self.new_folder_path(name)?;

		match fs::create_dir(&path) {
			Ok(()) => {
				self.set_status(format!("Created folder {}", path.display()));
				Ok(())
			}
			Err(error) => {
				let message = format!("Failed to create folder {}: {error}", path.display());
				self.set_status(message.clone());
				Err(message)
			}
		}
	}

	fn create_csharp_script(&mut self, name: &str) -> Result<(), String> {
		let file_name = format!("{name}.cs");
		let path = self.new_file_path(&file_name)?;
		let class_name = name;

		let template = format!(
			r"using ZeroEngine;

public class {class_name} : ZEScript
{{
	// Fields

	public override void OnCreate()
	{{
	}}

	public override void OnStart()
	{{
	}}

	public override void OnUpdate()
	{{
	}}

	public override void OnFixedUpdate()
	{{
	}}
}}
"
		);

		match fs::write(&path, template) {
			Ok(()) => {
				self.set_status(format!("Created script {}", path.display()));
				Ok(())
			}
			Err(error) => {
				let message = format!("Failed to create script {}: {error}", path.display());
				self.set_status(message.clone());
				Err(message)
			}
		}
	}

	fn create_csharp_component(&mut self, name: &str) -> Result<(), String> {
		let file_name = format!("{name}.cs");
		let path = self.new_file_path(&file_name)?;
		let class_name = name;

		let template = format!(
			r"using ZeroEngine;

public class {class_name} : ZEScript
{{
	// Fields
}}
"
		);

		match fs::write(&path, template) {
			Ok(()) => {
				self.set_status(format!("Created component {}", path.display()));
				Ok(())
			}
			Err(error) => {
				let message = format!("Failed to create component {}: {error}", path.display());
				self.set_status(message.clone());
				Err(message)
			}
		}
	}

	fn create_shader(&mut self, name: &str) -> Result<(), String> {
		let file_name = format!("{name}.wesl");
		let path = self.new_file_path(&file_name)?;

		let template = r"@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) vec4<f32> {
	return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
	return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
"
		.to_string();

		match fs::write(&path, template) {
			Ok(()) => {
				self.set_status(format!("Created shader {}", path.display()));
				Ok(())
			}
			Err(error) => {
				let message = format!("Failed to create shader {}: {error}", path.display());
				self.set_status(message.clone());
				Err(message)
			}
		}
	}

	/// Unlike the other `create_*` helpers, this writes under a fixed
	/// `<assets_root>/animation_clips/` folder with numeric-suffix collision
	/// handling, mirroring `TextureSheetCreateDialog::create` rather than
	/// `new_file_path`'s "wherever the explorer is currently browsing"
	/// behavior -- the source sheet (not the current directory) is what
	/// meaningfully identifies where a clip belongs.
	fn create_animation_clip(
		&mut self,
		name: &str,
		source_sheet: Option<PathBuf>,
		context: &mut EditorPanelContext<'_>,
	) -> Result<(), String> {
		let source_sheet = source_sheet.ok_or_else(|| "No source texture sheet selected.".to_string())?;
		let project = context.project.ok_or_else(|| "No project loaded.".to_string())?;
		let assets_root = &project.asset_dir;

		let relative_sheet = super::texture_sheet_common::relativize_asset_path(assets_root, &source_sheet)
			.ok_or_else(|| "Source sheet is not inside the project's asset directory.".to_string())?;

		let clip = ze_assets::AnimationClip::new(ze_assets::AssetRef::game(relative_sheet));

		let dir = assets_root.join("animation_clips");
		let mut dest = dir.join(format!("{name}.{}", ze_assets::ANIMATION_CLIP_EXTENSION));
		let mut suffix = 2;
		while dest.exists() {
			dest = dir.join(format!("{name}_{suffix}.{}", ze_assets::ANIMATION_CLIP_EXTENSION));
			suffix += 1;
		}

		clip.save(&dest)
			.map_err(|error| format!("Failed to save animation clip: {error}"))?;
		let dest = dest.canonicalize().unwrap_or(dest);

		self.set_status(format!("Created animation clip {}", dest.display()));
		*context.editor_request = Some(crate::editor_workspace::EditorRequest::OpenAnimationClipAsset(dest));

		Ok(())
	}

	fn create_zeignore_if_missing(&mut self) {
		let path = self.current_path.join(".zeignore");
		if path.exists() {
			self.set_status("'.zeignore' already exists");
			return;
		}
		match fs::write(&path, "") {
			Ok(()) => self.set_status(format!("Created {}", path.display())),
			Err(error) => self.set_status(format!("Failed to create .zeignore: {error}")),
		}
	}

	fn create_gitignore_if_missing(&mut self) {
		let path = self.current_path.join(".gitignore");
		if path.exists() {
			self.set_status("'.gitignore' already exists");
			return;
		}
		match fs::write(&path, "") {
			Ok(()) => self.set_status(format!("Created {}", path.display())),
			Err(error) => self.set_status(format!("Failed to create .gitignore: {error}")),
		}
	}

	// ---------------------------------------------------------------------------
	// Clipboard operations
	// ---------------------------------------------------------------------------

	fn cut_selected(&mut self) {
		let Some(name) = self.selected_entry.as_ref() else {
			return;
		};
		let source = self.current_path.join(name);
		if !source.exists() {
			return;
		}

		self.clipboard = Some(ClipboardEntry {
			source: source.canonicalize().unwrap_or(source),
			cut: true,
		});
	}

	fn copy_selected(&mut self) {
		let Some(name) = self.selected_entry.as_ref() else {
			return;
		};
		let source = self.current_path.join(name);
		if !source.exists() {
			return;
		}

		self.clipboard = Some(ClipboardEntry {
			source: source.canonicalize().unwrap_or(source),
			cut: false,
		});
	}

	fn paste(&mut self) {
		let Some(entry) = self.clipboard.as_ref() else {
			return;
		};

		let is_cut = entry.cut;
		let Some(name) = entry.source.file_name().and_then(|n| n.to_str()) else {
			return;
		};

		let dest = self.resolve_paste_destination(name);
		let result = if entry.cut {
			fs::rename(&entry.source, &dest)
		} else {
			fs::copy(&entry.source, &dest).map(|_| ())
		};

		match result {
			Ok(()) => {
				if is_cut {
					self.clipboard = None;
				}
				self.set_status(format!("Pasted {}", dest.display()));
			}
			Err(error) => {
				self.set_status(format!("Failed to paste: {error}"));
			}
		}
	}

	fn resolve_paste_destination(&self, name: &str) -> PathBuf {
		let dest = self.current_path.join(name);
		if !dest.exists() {
			return dest;
		}

		// Append _copy before extension
		let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
		let ext = Path::new(name).extension().and_then(|s| s.to_str());

		let mut counter = 1;
		loop {
			let new_name = ext.map_or_else(
				|| format!("{stem}_copy{counter}"),
				|ext| format!("{stem}_copy{counter}.{ext}"),
			);
			let candidate = self.current_path.join(&new_name);
			if !candidate.exists() {
				return candidate;
			}
			counter += 1;
		}
	}

	// ---------------------------------------------------------------------------
	// Delete / rename
	// ---------------------------------------------------------------------------

	fn delete_selected(&mut self) {
		let Some(name) = self.selected_entry.as_ref() else {
			return;
		};
		let path = self.current_path.join(name);
		if !path.exists() {
			return;
		}
		if self.is_protected_path(&path) {
			self.set_status(format!("Cannot delete protected path {}", path.display()));
			return;
		}

		let confirmed = rfd::MessageDialog::new()
			.set_level(rfd::MessageLevel::Warning)
			.set_title("Confirm Delete")
			.set_description(format!("Delete '{name}'? This cannot be undone."))
			.set_buttons(rfd::MessageButtons::YesNo)
			.show();

		if confirmed != rfd::MessageDialogResult::Yes {
			return;
		}

		let result = if path.is_dir() {
			fs::remove_dir_all(&path)
		} else {
			fs::remove_file(&path)
		};

		match result {
			Ok(()) => {
				self.selected_entry = None;
				self.set_status(format!("Deleted {}", path.display()));
			}
			Err(error) => {
				self.set_status(format!("Failed to delete {}: {error}", path.display()));
			}
		}
	}

	fn begin_rename(&mut self) { self.renaming = self.selected_entry.clone(); }

	fn commit_rename(&mut self, new_name: &str) {
		let Some(old_name) = self.selected_entry.as_ref() else {
			self.renaming = None;
			return;
		};

		let new_name = new_name.trim().to_string();
		if new_name.is_empty() || new_name == *old_name {
			self.renaming = None;
			return;
		}

		let src = self.current_path.join(old_name);
		let dst = self.current_path.join(&new_name);

		if dst.exists() {
			self.set_status(format!("Cannot rename: '{new_name}' already exists"));
			self.renaming = None;
			return;
		}

		match fs::rename(&src, &dst) {
			Ok(()) => {
				self.selected_entry = Some(new_name);
				self.set_status(format!("Renamed {} to {}", src.display(), dst.display()));
			}
			Err(error) => {
				self.set_status(format!("Failed to rename: {error}"));
			}
		}

		self.renaming = None;
	}

	// ---------------------------------------------------------------------------
	// Input handling
	// ---------------------------------------------------------------------------

	fn handle_mouse_navigation(&mut self, ui: &Ui, panel_hovered: bool) {
		if !panel_hovered {
			return;
		}

		if ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Extra1)) {
			self.go_back();
		}
		if ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Extra2)) {
			self.go_forward();
		}
	}

	fn handle_global_input(&mut self, ui: &Ui) {
		if ui.ctx().egui_wants_keyboard_input() {
			return;
		}
		// Clipboard shortcuts
		if ui.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::X)) {
			self.cut_selected();
		}
		if ui.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::C)) {
			self.copy_selected();
		}
		if ui.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::V)) {
			self.paste();
		}

		// Grid keyboard navigation — only when grid has focus
		if !self.grid_focused {
			return;
		}

		if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
			let Some(name) = self.selected_entry.as_ref() else {
				return;
			};
			let path = self.current_path.join(name);
			if path.is_dir() {
				self.open_directory(path);
			}
		}

		let entries = self.read_entries_for_grid();
		if entries.is_empty() {
			return;
		}

		let columns = Self::grid_columns(ui);
		let current_index = self
			.selected_entry
			.as_ref()
			.and_then(|sel| entries.iter().position(|(n, _)| n == sel));

		let new_index = if ui.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
			current_index.map(|i| (i + 1).min(entries.len() - 1))
		} else if ui.input(|input| input.key_pressed(egui::Key::ArrowLeft)) {
			current_index.map(|i| i.saturating_sub(1))
		} else if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
			current_index.map(|i| (i + columns).min(entries.len() - 1))
		} else if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
			current_index.map(|i| i.saturating_sub(columns))
		} else {
			None
		};

		if let Some(index) = new_index {
			self.selected_entry = Some(entries[index].0.clone());
		}
	}

	fn read_entries_for_grid(&self) -> Vec<(String, bool)> {
		let ignore_patterns = self.load_ignore_patterns();

		#[allow(clippy::option_if_let_else)]
		let mut result: Vec<(String, bool)> = match std::fs::read_dir(&self.current_path) {
			Ok(entries) => entries
				.filter_map(Result::ok)
				.filter(|entry| {
					let path = entry.path();
					!Self::matches_ignore_pattern(&path, &ignore_patterns)
				})
				.map(|entry| {
					let path = entry.path();
					let name = path
						.file_name()
						.and_then(|n| n.to_str().map(String::from))
						.unwrap_or_default();
					(name, path.is_dir())
				})
				.collect(),
			Err(_) => Vec::new(),
		};

		result.sort_by(|(a_name, a_dir), (b_name, b_dir)| {
			// Directories first, then alphabetical
			if *a_dir != *b_dir {
				return b_dir.cmp(a_dir); // dirs come first (false < true in sort)
			}
			a_name.cmp(b_name)
		});

		result
	}

	fn grid_columns(ui: &Ui) -> usize {
		let avail = ui.available_width();
		let col = ((avail + TILE_SPACING) / (TILE_SIZE.x + TILE_SPACING)).floor() as usize;
		col.max(1)
	}

	// ---------------------------------------------------------------------------
	// Left half: Favorites + Folder tree
	// ---------------------------------------------------------------------------

	fn show_left_panel(&mut self, ui: &mut Ui) {
		ui.set_width(ui.available_width());

		egui::ScrollArea::vertical()
			.id_salt("file_left_panel_scroll")
			.auto_shrink([false, false])
			.show(ui, |ui| {
				if !self.favorites.is_empty() {
					ui.label(
						egui::RichText::new("FAVORITES")
							.size(10.0)
							.color(egui::Color32::from_rgb(100, 100, 110)),
					);
					ui.add_space(2.0);

					for fav in self.favorites.clone() {
						ui.horizontal(|ui| {
							ui.add_space(18.0);

							ui.add_sized(
								egui::vec2(11.0, 11.0),
								egui::Label::new(
									egui::RichText::new("\u{F024B}")
										.size(11.0)
										.color(Self::entry_icon_color(true)),
								),
							);

							let fav_color = if fav == self.current_path {
								egui::Color32::from_rgb(83, 154, 252)
							} else {
								Self::entry_text_color(true)
							};

							let fav_name = fav.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();

							let fav_resp = ui.add_sized(
								egui::vec2((ui.available_width() - 15.0).max(20.0), 18.0),
								egui::Label::new(egui::RichText::new(&fav_name).size(12.0).color(fav_color)),
							);
							if fav_resp.clicked() {
								self.open_directory(fav.clone());
							}

							let star_resp = ui.add_sized(
								egui::vec2(11.0, 11.0),
								egui::Button::new(
									egui::RichText::new("\u{2605}")
										.size(11.0)
										.color(egui::Color32::from_rgb(255, 200, 50)),
								)
								.frame(false),
							);
							if star_resp.clicked() {
								self.toggle_favorite(&fav);
							}
						});
					}

					ui.add_space(2.0);
					ui.separator();
					ui.add_space(2.0);
				}

				self.show_directory_tree(ui, self.project_root.clone(), 0);
			});
	}

	fn is_favorite(&self, path: &Path) -> bool { self.favorites.iter().any(|f| f == path) }

	fn toggle_favorite(&mut self, path: &Path) {
		let canonical = Self::normalize_directory_path(path.to_path_buf());
		if let Some(pos) = self.favorites.iter().position(|f| f == &canonical) {
			self.favorites.remove(pos);
		} else {
			self.favorites.push(canonical);
		}
		self.mark_favorites_dirty();
	}

	#[allow(clippy::needless_pass_by_value)]
	fn show_directory_tree(&mut self, ui: &mut Ui, dir: PathBuf, depth: usize) {
		let indent = depth as f32 * 16.0;
		let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("/").to_string();
		let is_expanded = self.tree_expanded.contains(&dir);
		let is_selected = dir == self.current_path;

		let ignore_patterns = self.load_ignore_patterns_for(&dir);
		let mut children = Self::read_subdirectories(&dir);
		children.retain(|child| self.is_within_project_root(child));
		children.retain(|child| !Self::matches_ignore_pattern(child, &ignore_patterns));
		children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
		let has_children = !children.is_empty();

		ui.horizontal(|ui| {
			ui.add_space(indent);

			if has_children {
				let triangle = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
				let tri_resp = ui.add_sized(egui::vec2(12.0, 16.0), egui::Button::new(triangle).frame(false));

				if tri_resp.clicked() {
					if is_expanded {
						self.tree_expanded.remove(&dir);
					} else {
						self.tree_expanded.insert(dir.clone());
					}
					self.mark_tree_expanded_dirty();
				}
			} else {
				ui.add_space(12.0);
			}

			let icon_color = Self::entry_icon_color(true);
			ui.add_sized(
				egui::vec2(16.0, 16.0),
				egui::Label::new(egui::RichText::new("\u{F024B}").size(12.0).color(icon_color)),
			);

			let text_color = if is_selected {
				egui::Color32::from_rgb(83, 154, 252)
			} else {
				Self::entry_text_color(true)
			};

			let node_resp = ui.add_sized(
				egui::vec2(ui.available_width() - 20.0, 18.0).max(egui::vec2(20.0, 18.0)),
				egui::Label::new(egui::RichText::new(&name).size(12.0).color(text_color)),
			);

			if node_resp.clicked() {
				self.open_directory(dir.clone());
			}

			// Star button for favorites, right-aligned
			let is_fav = self.is_favorite(&dir);
			let star = if is_fav { "\u{2605}" } else { "\u{2606}" };
			let star_color = if is_fav {
				egui::Color32::from_rgb(255, 200, 50)
			} else {
				egui::Color32::from_rgb(100, 100, 100)
			};
			let star_resp = ui.add_sized(
				egui::vec2(16.0, 16.0),
				egui::Button::new(egui::RichText::new(star).size(12.0).color(star_color)).frame(false),
			);
			if star_resp.clicked() {
				self.toggle_favorite(&dir);
			}
		});

		if is_expanded {
			for child in children {
				self.show_directory_tree(ui, child, depth + 1);
			}
		}
	}

	// ---------------------------------------------------------------------------
	// Right half: Content grid + Status
	// ---------------------------------------------------------------------------

	#[allow(deprecated)]
	fn show_right_panel(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		let panel_rect = ui.max_rect();

		let content_rect = egui::Rect::from_min_size(
			egui::pos2(panel_rect.left(), panel_rect.top()),
			egui::vec2(panel_rect.width(), panel_rect.height()),
		);

		// Content grid
		{
			let mut content_ui = ui.child_ui(content_rect, egui::Layout::top_down(egui::Align::LEFT), None);
			egui::ScrollArea::vertical()
				.auto_shrink([false, false])
				.show(&mut content_ui, |ui| {
					self.show_content_grid(ui, context);
				});
		}
	}

	// ---------------------------------------------------------------------------
	// Content grid (tiles)
	// ---------------------------------------------------------------------------

	fn show_content_grid(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		let entries = self.read_entries_for_grid();

		if entries.is_empty() {
			ui.vertical_centered(|ui| {
				ui.add_space(40.0);
				ui.label(
					egui::RichText::new("Empty folder")
						.size(14.0)
						.color(egui::Color32::from_rgb(100, 100, 100)),
				);
			});

			// Right-click on empty space
			let resp = ui.allocate_response(
				egui::vec2(ui.available_width(), ui.available_height()),
				egui::Sense::click(),
			);
			resp.context_menu(|ui| {
				self.show_empty_context_menu(ui);
			});
			if resp.clicked() {
				self.selected_entry = None;
				self.grid_focused = true;
			}
			return;
		}

		let columns = Self::grid_columns(ui).max(1);

		for chunk in entries.chunks(columns) {
			ui.horizontal(|ui| {
				for (name, is_dir) in chunk {
					self.render_tile(ui, name, *is_dir, context);
				}
				// Fill remaining space to allow right-click in empty area
				let remaining = ui.available_width();
				if remaining > 0.0 {
					let resp = ui.allocate_response(egui::vec2(remaining, TILE_SIZE.y), egui::Sense::click());
					if resp.clicked() {
						self.selected_entry = None;
						self.grid_focused = true;
					}
					resp.context_menu(|ui| {
						self.show_empty_context_menu(ui);
					});
				}
			});
		}

		// Fill remaining vertical space for context menu on empty area
		let remaining_height = ui.available_height().max(24.0);
		let empty_resp = ui.allocate_response(egui::vec2(ui.available_width(), remaining_height), egui::Sense::click());
		if empty_resp.clicked() {
			self.selected_entry = None;
			self.grid_focused = true;
		}
		empty_resp.context_menu(|ui| {
			self.show_empty_context_menu(ui);
		});
	}

	fn render_tile(&mut self, ui: &mut Ui, name: &str, is_directory: bool, context: &mut EditorPanelContext<'_>) {
		let is_selected = self.selected_entry.as_deref() == Some(name);
		let is_cut = self
			.clipboard
			.as_ref()
			.is_some_and(|c| c.cut && c.source.file_name().and_then(|n| n.to_str()) == Some(name));

		let path = self.current_path.join(name);
		let icon = Self::entry_icon(&path, is_directory);
		let icon_color = Self::entry_icon_color(is_directory);
		let text_color = Self::entry_text_color(is_directory);

		let (rect, response) = ui.allocate_exact_size(TILE_SIZE, egui::Sense::click());

		// Background fill
		let bg = if is_selected {
			egui::Color32::from_rgb(40, 70, 130)
		} else if response.hovered() {
			egui::Color32::from_rgb(40, 40, 40)
		} else {
			egui::Color32::from_rgb(24, 24, 24)
		};

		let mut bg = egui::Rgba::from(bg);
		if is_cut {
			bg = bg * egui::Rgba::from_rgba_premultiplied(1.0, 1.0, 1.0, 0.4);
		}

		let painter = ui.painter();
		painter.rect_filled(rect, CornerRadius::same(6), bg);

		if is_selected {
			painter.rect_stroke(
				rect,
				CornerRadius::same(6),
				egui::Stroke::new(1.5, egui::Color32::from_rgb(83, 154, 252)),
				egui::StrokeKind::Middle,
			);
		}

		// Icon in upper portion
		let icon_pos = egui::pos2(rect.center().x, rect.top() + 30.0);
		painter.text(
			icon_pos,
			egui::Align2::CENTER_CENTER,
			icon,
			egui::FontId::proportional(28.0),
			icon_color,
		);

		// Name in lower portion
		let name_rect = egui::Rect::from_min_size(
			egui::pos2(rect.left() + 2.0, rect.top() + 70.0),
			egui::vec2(rect.width() - 4.0, rect.height() - 72.0),
		);

		let name_clip_painter = painter.with_clip_rect(name_rect);
		let font = egui::FontId::proportional(10.0);
		let galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap(name.into(), font.clone(), text_color));

		// Truncate with ellipsis if too wide
		let galley = if galley.size().x > name_rect.width() {
			let truncated: String = name.chars().take(10).collect();
			let display = if truncated.len() < name.len() {
				format!("{}...", &truncated)
			} else {
				name.to_string()
			};
			ui.fonts_mut(|fonts| fonts.layout_no_wrap(display, font, text_color))
		} else {
			galley
		};

		let text_pos = egui::pos2(name_rect.center().x, name_rect.center().y);
		name_clip_painter.galley(text_pos, galley, text_color);

		// Interactions
		if response.double_clicked() {
			if is_directory {
				self.open_directory(path.clone());
			}
		} else if response.clicked() {
			self.selected_entry = Some(name.to_string());
			self.grid_focused = true;
			if !is_directory {
				self.inspect_file(context, &path);
			}
		}

		if response.double_clicked() && !is_directory {
			self.inspect_file(context, &path);
		}

		// Context menu
		response.context_menu(|ui| {
			self.show_item_context_menu(ui, &path, is_directory, context);
		});
	}

	// ---------------------------------------------------------------------------
	// Status bar
	// ---------------------------------------------------------------------------

	fn show_item_context_menu(
		&mut self,
		ui: &mut Ui,
		path: &Path,
		is_directory: bool,
		context: &EditorPanelContext<'_>,
	) {
		if !is_directory
			&& crate::asset_hot_reload::is_texture_asset(&path.to_string_lossy())
			&& let Some(project) = context.project
		{
			if ui.button("Create Texture Sheet").clicked() {
				self.pending_texture_sheet_create =
					Some(TextureSheetCreateDialog::new(path.to_path_buf(), &project.asset_dir));
				ui.close();
			}
			ui.separator();
		}

		if !is_directory
			&& path
				.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(|name| name.ends_with(&format!(".{}", ze_assets::TEXTURE_SHEET_EXTENSION)))
		{
			if ui.button("Create Animation Clip").clicked() {
				self.pending_create = Some(PendingCreate::for_animation_clip(path.to_path_buf()));
				ui.close();
			}
			ui.separator();
		}

		self.show_create_submenu(ui);
		ui.separator();
		if ui
			.add_enabled(self.selected_entry.is_some(), egui::Button::new("Delete"))
			.clicked()
		{
			self.delete_selected();
			ui.close();
		}
		if ui
			.add_enabled(self.selected_entry.is_some(), egui::Button::new("Rename"))
			.clicked()
		{
			self.begin_rename();
			ui.close();
		}
		if is_directory && ui.button("Add to Favorites").clicked() {
			let canonical = Self::normalize_directory_path(path.to_path_buf());
			if !self.favorites.contains(&canonical) {
				self.favorites.push(canonical);
				self.mark_favorites_dirty();
			}
			ui.close();
		}
		ui.separator();
		if ui.button("Cut").clicked() {
			self.cut_selected();
			ui.close();
		}
		if ui.button("Copy").clicked() {
			self.copy_selected();
			ui.close();
		}
		if ui
			.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste"))
			.clicked()
		{
			self.paste();
			ui.close();
		}
	}

	fn show_empty_context_menu(&mut self, ui: &mut Ui) {
		self.show_create_submenu(ui);
		ui.separator();
		if ui
			.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste"))
			.clicked()
		{
			self.paste();
			ui.close();
		}
	}

	fn show_create_submenu(&mut self, ui: &mut Ui) {
		ui.menu_button("Create", |ui| {
			if ui.button("C# Script").clicked() {
				self.pending_create = Some(PendingCreate::new(PendingCreateKind::CSharpScript));
				ui.close();
			}
			if ui.button("C# Component").clicked() {
				self.pending_create = Some(PendingCreate::new(PendingCreateKind::CSharpComponent));
				ui.close();
			}
			if ui.button("Scene").clicked() {
				self.pending_create = Some(PendingCreate::new(PendingCreateKind::Scene));
				ui.close();
			}
			if ui.button("Shader").clicked() {
				self.pending_create = Some(PendingCreate::new(PendingCreateKind::Shader));
				ui.close();
			}
			if ui.button("Folder").clicked() {
				self.pending_create = Some(PendingCreate::new(PendingCreateKind::Folder));
				ui.close();
			}
			if ui.button("New .zeignore").clicked() {
				self.create_zeignore_if_missing();
				ui.close();
			}
			if ui.button("New .gitignore").clicked() {
				self.create_gitignore_if_missing();
				ui.close();
			}
		});
	}

	// ---------------------------------------------------------------------------
	// Create / delete popups
	// ---------------------------------------------------------------------------

	fn confirm_pending_create(&mut self, context: &mut EditorPanelContext<'_>) {
		let Some(pending) = self.pending_create.as_ref() else {
			return;
		};
		let kind = pending.kind;
		let name = pending.name.clone();
		let source_sheet = pending.source_sheet.clone();

		if name.trim().is_empty() {
			if let Some(p) = self.pending_create.as_mut() {
				p.error = Some("Name cannot be empty".to_string());
			}
			return;
		}

		let result = match kind {
			PendingCreateKind::Folder => self.create_folder(&name),
			PendingCreateKind::CSharpScript => self.create_csharp_script(&name),
			PendingCreateKind::CSharpComponent => self.create_csharp_component(&name),
			PendingCreateKind::Shader => self.create_shader(&name),
			PendingCreateKind::Scene => {
				*context.editor_request = Some(crate::editor_workspace::EditorRequest::CreateScene(name));
				Ok(())
			}
			PendingCreateKind::AnimationClip => self.create_animation_clip(&name, source_sheet, context),
		};

		match result {
			Ok(()) => {
				self.pending_create = None;
			}
			Err(error) => {
				if let Some(pending) = self.pending_create.as_mut() {
					pending.error = Some(error);
				}
			}
		}
	}

	fn cancel_pending_create(&mut self) { self.pending_create = None; }

	fn show_pending_create_popup(&mut self, ui: &Ui, context: &mut EditorPanelContext<'_>) {
		let Some(pending) = self.pending_create.as_mut() else {
			return;
		};

		let title = pending.kind.title();
		let prompt = pending.kind.prompt();
		let input_id = ui.id().with(("file_explorer_pending_create", title));
		let mut confirm = false;
		let mut cancel = false;

		egui::Window::new(title)
			.id(ui.id().with("file_explorer_create_popup"))
			.collapsible(false)
			.resizable(false)
			.default_width(260.0)
			.anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
			.show(ui.ctx(), |ui| {
				ui.label(prompt);
				let response = ui.add(
					egui::TextEdit::singleline(&mut pending.name)
						.id(input_id)
						.desired_width(f32::INFINITY),
				);
				response.request_focus();

				if let Some(error) = &pending.error {
					ui.colored_label(egui::Color32::YELLOW, error);
				}

				ui.horizontal(|ui| {
					if ui.button("Create").clicked() {
						confirm = true;
					}
					if ui.button("Cancel").clicked() {
						cancel = true;
					}
				});

				confirm |= response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
				cancel |= ui.input(|input| input.key_pressed(egui::Key::Escape));
			});

		if confirm {
			self.confirm_pending_create(context);
		} else if cancel {
			self.cancel_pending_create();
		}
	}

	fn show_pending_texture_sheet_create(&mut self, ui: &Ui, context: &mut EditorPanelContext<'_>) {
		let Some(dialog) = self.pending_texture_sheet_create.as_mut() else {
			return;
		};
		let Some(project) = context.project else {
			self.pending_texture_sheet_create = None;
			return;
		};

		match dialog.show(ui.ctx(), &project.asset_dir) {
			TextureSheetCreateOutcome::Pending => {}
			TextureSheetCreateOutcome::Cancelled => self.pending_texture_sheet_create = None,
			TextureSheetCreateOutcome::Created(new_path) => {
				self.pending_texture_sheet_create = None;
				*context.editor_request = Some(crate::editor_workspace::EditorRequest::OpenTextureSheetAsset(new_path));
			}
		}
	}

	fn show_rename_popup(&mut self, ui: &Ui) {
		let Some(name) = self.renaming.as_mut() else {
			return;
		};

		let mut confirmed = false;
		let mut cancelled = false;
		let input_id = ui.id().with("file_explorer_rename");

		egui::Window::new("Rename")
			.id(ui.id().with("file_explorer_rename_popup"))
			.collapsible(false)
			.resizable(false)
			.default_width(260.0)
			.anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
			.show(ui.ctx(), |ui| {
				ui.label("New name");
				let response = ui.add(
					egui::TextEdit::singleline(name)
						.id(input_id)
						.desired_width(f32::INFINITY),
				);
				response.request_focus();

				ui.horizontal(|ui| {
					if ui.button("Rename").clicked() {
						confirmed = true;
					}
					if ui.button("Cancel").clicked() {
						cancelled = true;
					}
				});

				confirmed |= response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
				cancelled |= ui.input(|input| input.key_pressed(egui::Key::Escape));
			});

		if confirmed {
			let new_name = self.renaming.take().unwrap_or_default();
			self.commit_rename(&new_name);
		} else if cancelled {
			self.renaming = None;
		}
	}

	fn show_status_bar(&mut self, ui: &mut Ui) {
		let show_flash = self
			.status_flash
			.as_ref()
			.is_some_and(|(_, time)| time.elapsed() < std::time::Duration::from_secs(3));

		if !show_flash {
			self.status_flash = None;
		}

		let text = if let Some((msg, _)) = &self.status_flash {
			msg.clone()
		} else {
			let root_name = self
				.project_root
				.file_name()
				.and_then(|n| n.to_str())
				.unwrap_or("Project");

			let relative = self
				.current_path
				.strip_prefix(&self.project_root)
				.unwrap_or(&self.current_path);

			if relative.as_os_str().is_empty() {
				format!("{root_name}/")
			} else {
				format!("{root_name}/{}", relative.display())
			}
		};

		ui.add_space(6.0);
		ui.label(
			egui::RichText::new(text)
				.size(12.0)
				.color(egui::Color32::from_rgb(160, 170, 185)),
		);
	}
}

const FAVORITES_DIR: &str = ".zeroeditor/favorites_folders.toml";
const TREE_EXPANDED_DIR: &str = ".zeroeditor/tree_expanded.toml";

impl Panel for FileExplorer {
	fn name(&self) -> &'static str { "File Explorer" }

	#[allow(deprecated)]
	fn show(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		self.ensure_current_path_is_valid();
		self.handle_global_input(ui);

		let panel_rect = ui.max_rect();
		let panel_hovered = ui.rect_contains_pointer(panel_rect);
		self.handle_mouse_navigation(ui, panel_hovered);

		ui.spacing_mut().item_spacing = egui::vec2(4.0, 1.0);
		ui.spacing_mut().button_padding = egui::vec2(4.0, 0.0);
		ui.spacing_mut().interact_size.y = 16.0;

		egui::Panel::left("file_browser_tree")
			.resizable(true)
			.default_size(LEFT_PANEL_WIDTH)
			.min_size(120.0)
			.frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(18, 18, 18)))
			.show_inside(ui, |ui| {
				self.show_left_panel(ui);
			});

		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(18, 18, 18)))
			.show_inside(ui, |ui| {
				self.show_right_panel(ui, context);
			});

		// Reserve a strip at the very bottom of the File Explorer panel
		let status_height = 14.0;
		let full_rect = ui.max_rect();
		let status_rect = egui::Rect::from_min_size(
			egui::pos2(full_rect.left(), full_rect.bottom() - status_height),
			egui::vec2(full_rect.width(), status_height),
		);

		let mut status_ui = ui.child_ui(status_rect, egui::Layout::left_to_right(egui::Align::Center), None);
		status_ui.set_height(status_height);

		// Background fill
		status_ui.painter().rect_filled(
			status_rect,
			egui::CornerRadius::ZERO,
			egui::Color32::from_rgb(15, 15, 15),
		);

		self.show_status_bar(&mut status_ui);

		self.show_pending_create_popup(ui, context);
		self.show_rename_popup(ui);
		self.show_pending_texture_sheet_create(ui, context);
		self.save_favorites();
		self.save_tree_expanded();
	}
}
