use std::{
	fs,
	path::{Path, PathBuf},
	time::SystemTime,
};

use egui::{
	ColorImage, PopupCloseBehavior, TextBuffer, TextEdit, TextureOptions, Ui,
	containers::menu::{MenuConfig, SubMenuButton},
};
use ze_assets::AssetRef;
use ze_core::{Quat, Vec2, Vec3};
use ze_ecs::{
	EditorOnly, EntityId, Name, Parent, SaveFile, Tag, Transform, TransformInheritance,
	components::{
		AudioSource, Collider, ColliderShape, CollisionDetection, Inactive, PhysicsSettings, RigidBody, RigidBodyType,
	},
};
use ze_project::{GameData, PROJECT_FILE_NAME, Project};
use ze_renderer::{
	Camera, CameraProjection, Sprite, SpriteColorMode, SpriteColorSettings, SpriteSettings, SpriteSize, TextureSource,
};
use ze_scripting_cs::{
	Script as ScriptData, ScriptFieldMetadata, ScriptFieldValue, ScriptingSystem, Scripts as ZeScripts,
};
use ze_ui::{UIBar, UIButton, UIRect, UIText};

use super::{EditorPanelContext, EditorSelection, Panel};
use crate::undo_redo::SceneSnapshotCommand;

pub struct InspectorPanel {
	code_buffer: String,
	code_buffer_snapshot: String,
	opened_code_path: Option<PathBuf>,
	project_settings: Option<ProjectSettingsState>,
	project_settings_error_path: Option<PathBuf>,
	project_settings_error_message: Option<String>,
	pending_scene_edit: Option<PendingSceneEdit>,
	component_search: String,
	preview_cache: Option<(PathBuf, SystemTime, egui::TextureHandle)>,
	ignore_new_pattern: String,
	new_tag_buffer: String,
	show_add_tag_popup: bool,
	script_classes_cache: Option<Vec<String>>,
}

#[derive(Debug)]
struct PendingSceneEdit {
	before: SaveFile,
	dirty: bool,
}

#[derive(Debug, Default)]
struct InspectorContextMenuAction {
	add_component: Option<InspectableComponent>,
	add_script: Option<String>,
	remove_component: bool,
	remove_script_path: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
struct FieldEdit {
	changed: bool,
	committed: bool,
}

impl FieldEdit {
	const fn changed(changed: bool) -> Self {
		Self {
			changed,
			committed: changed,
		}
	}

	const fn include(&mut self, other: Self) {
		self.changed |= other.changed;
		self.committed |= other.committed;
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectableComponent {
	Tag,
	Transform,
	Inactive,
	Sprite,
	Camera,
	RigidBody,
	Collider,
	PhysicsSettings,
	EditorOnly,
	UIButton,
	UIBar,
	UIText,
	AudioSource,
}

impl InspectableComponent {
	const ADDABLE: [Self; 12] = [
		Self::Tag,
		Self::Transform,
		Self::Inactive,
		Self::Sprite,
		Self::Camera,
		Self::RigidBody,
		Self::Collider,
		Self::EditorOnly,
		Self::UIButton,
		Self::UIBar,
		Self::UIText,
		Self::AudioSource,
	];

	const fn label(self) -> &'static str {
		match self {
			Self::Tag => "Tag",
			Self::Transform => "Transform",
			Self::Inactive => "Inactive",
			Self::Sprite => "Sprite",
			Self::Camera => "Camera",
			Self::RigidBody => "Rigidbody",
			Self::Collider => "Collider",
			Self::PhysicsSettings => "PhysicsSettings",
			Self::EditorOnly => "EditorOnly",
			Self::UIButton => "UI Button",
			Self::UIBar => "UI Bar",
			Self::UIText => "UI Text",
			Self::AudioSource => "Audio Source",
		}
	}

	fn is_present(self, scene: &ze_ecs::Scene, entity: EntityId) -> bool {
		match self {
			Self::Tag => scene.world().get::<&Tag>(entity).is_ok(),
			Self::Transform => scene.world().get::<&Transform>(entity).is_ok(),
			Self::Inactive => scene.world().get::<&Inactive>(entity).is_ok(),
			Self::Sprite => scene.world().get::<&Sprite>(entity).is_ok(),
			Self::Camera => scene.world().get::<&Camera>(entity).is_ok(),
			Self::RigidBody => scene.world().get::<&RigidBody>(entity).is_ok(),
			Self::Collider => scene.world().get::<&Collider>(entity).is_ok(),
			Self::PhysicsSettings => scene.world().get::<&PhysicsSettings>(entity).is_ok(),
			Self::EditorOnly => scene.world().get::<&EditorOnly>(entity).is_ok(),
			Self::UIButton => scene.world().get::<&UIButton>(entity).is_ok(),
			Self::UIBar => scene.world().get::<&UIBar>(entity).is_ok(),
			Self::UIText => scene.world().get::<&UIText>(entity).is_ok(),
			Self::AudioSource => scene.world().get::<&AudioSource>(entity).is_ok(),
		}
	}

	fn add_to(self, scene: &mut ze_ecs::Scene, entity: EntityId) {
		match self {
			Self::Tag => {
				scene.entity_mut(entity).add_component(Tag { tag: String::new() });
			}
			Self::Transform => {
				scene.entity_mut(entity).add_component(Transform::default());
			}
			Self::Inactive => {
				scene.entity_mut(entity).add_component(Inactive);
			}
			Self::Sprite => {
				scene.entity_mut(entity).add_component(default_sprite());
			}
			Self::Camera => {
				scene.entity_mut(entity).add_component(default_camera());
			}
			Self::RigidBody => {
				scene.entity_mut(entity).add_component(RigidBody::default());
			}
			Self::Collider => {
				scene.entity_mut(entity).add_component(Collider::default());
			}
			Self::PhysicsSettings => {
				scene.entity_mut(entity).add_component(PhysicsSettings::default());
			}
			Self::EditorOnly => {
				scene.entity_mut(entity).add_component(EditorOnly);
			}
			Self::UIButton => {
				scene.entity_mut(entity).add_component(UIButton {
					rect: UIRect {
						x: 0.0,
						y: 0.0,
						width: 100.0,
						height: 30.0,
					},
					text: "Button".to_string(),
					font_size: 14.0,
					color: [0.5, 0.5, 0.5, 1.0],
					hover_color: [0.6, 0.6, 0.6, 1.0],
					pressed_color: [0.35, 0.35, 0.35, 1.0],
					pressed: false,
					hovered: false,
					z_index: 0,
				});
			}
			Self::UIBar => {
				scene.entity_mut(entity).add_component(UIBar {
					rect: UIRect {
						x: 0.0,
						y: 0.0,
						width: 200.0,
						height: 20.0,
					},
					current: 50.0,
					max: 100.0,
					color: [0.0, 0.8, 0.2, 1.0],
					bg_color: [0.2, 0.2, 0.2, 1.0],
					text: None,
					z_index: 0,
				});
			}
			Self::UIText => {
				scene.entity_mut(entity).add_component(UIText {
					rect: UIRect {
						x: 0.0,
						y: 0.0,
						width: 200.0,
						height: 20.0,
					},
					text: "Text".to_string(),
					font_size: 14.0,
					color: [1.0, 1.0, 1.0, 1.0],
					z_index: 0,
				});
			}
			Self::AudioSource => {
				scene.entity_mut(entity).add_component(AudioSource::default());
			}
		}
	}

	fn remove_from(self, scene: &mut ze_ecs::Scene, entity: EntityId) {
		match self {
			Self::Tag => {
				let _ = scene.entity_mut(entity).remove_component::<Tag>();
			}
			Self::Transform => {
				let _ = scene.entity_mut(entity).remove_component::<Transform>();
			}
			Self::Inactive => {
				let _ = scene.entity_mut(entity).remove_component::<Inactive>();
			}
			Self::Sprite => {
				let _ = scene.entity_mut(entity).remove_component::<Sprite>();
			}
			Self::Camera => {
				let _ = scene.entity_mut(entity).remove_component::<Camera>();
			}
			Self::RigidBody => {
				let _ = scene.entity_mut(entity).remove_component::<RigidBody>();
			}
			Self::Collider => {
				let _ = scene.entity_mut(entity).remove_component::<Collider>();
			}
			Self::PhysicsSettings => {
				let _ = scene.entity_mut(entity).remove_component::<PhysicsSettings>();
			}
			Self::EditorOnly => {
				let _ = scene.entity_mut(entity).remove_component::<EditorOnly>();
			}
			Self::UIButton => {
				let _ = scene.entity_mut(entity).remove_component::<UIButton>();
			}
			Self::UIBar => {
				let _ = scene.entity_mut(entity).remove_component::<UIBar>();
			}
			Self::UIText => {
				let _ = scene.entity_mut(entity).remove_component::<UIText>();
			}
			Self::AudioSource => {
				let _ = scene.entity_mut(entity).remove_component::<AudioSource>();
			}
		}
	}
}

#[derive(Debug, Clone)]
struct ProjectSettingsState {
	path: PathBuf,
	project: Project,
	asset_dir_text: String,
	scripts_dll_text: String,
	icon_path_text: String,
	scenes: Vec<String>,
	game_data: GameData,
	status_message: Option<String>,
	dirty: bool,
	new_tag_buffer: String,
}

impl std::fmt::Debug for InspectorPanel {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("InspectorPanel")
			.field("opened_code_path", &self.opened_code_path)
			.field("project_settings", &self.project_settings)
			.field("project_settings_error_path", &self.project_settings_error_path)
			.field("project_settings_error_message", &self.project_settings_error_message)
			.field("pending_scene_edit", &self.pending_scene_edit)
			.field("component_search", &self.component_search)
			.field("preview_cache", &self.preview_cache.as_ref().map(|(p, t, _)| (p, t)))
			.finish_non_exhaustive()
	}
}

impl InspectorPanel {
	const FALLBACK_TAGS: [&'static str; 3] = ["Untagged", "Camera", "Player"];

	#[allow(clippy::missing_const_for_fn)]
	pub fn new() -> Self {
		Self {
			code_buffer: String::new(),
			code_buffer_snapshot: String::new(),
			opened_code_path: None,
			project_settings: None,
			project_settings_error_path: None,
			project_settings_error_message: None,
			pending_scene_edit: None,
			component_search: String::new(),
			preview_cache: None,
			ignore_new_pattern: String::new(),
			new_tag_buffer: String::new(),
			show_add_tag_popup: false,
			script_classes_cache: None,
		}
	}

	fn begin_scene_edit(&mut self, context: &EditorPanelContext<'_>) {
		if self.pending_scene_edit.is_none() {
			self.pending_scene_edit = Some(PendingSceneEdit {
				before: context.scene.snapshot(),
				dirty: false,
			});
		}
	}

	fn finish_scene_edit(&mut self, context: &mut EditorPanelContext<'_>) {
		let Some(pending) = self.pending_scene_edit.take() else {
			return;
		};
		if !pending.dirty {
			return;
		}
		let after = context.scene.snapshot();
		context
			.undo_redo
			.record_executed(Box::new(SceneSnapshotCommand::new(pending.before, after)));
	}

	const fn mark_scene_edit_dirty(&mut self) {
		if let Some(pending) = self.pending_scene_edit.as_mut() {
			pending.dirty = true;
		}
	}

	fn apply_field_edit(
		&mut self,
		context: &mut EditorPanelContext<'_>,
		field_edit: FieldEdit,
		edit: impl FnOnce(&mut ze_ecs::Scene),
	) {
		if field_edit.changed {
			self.begin_scene_edit(context);
			edit(context.scene);
			self.mark_scene_edit_dirty();
		}
		if field_edit.committed {
			self.finish_scene_edit(context);
		}
	}

	fn show_entity(&mut self, ui: &mut Ui, entity: EntityId, context: &mut EditorPanelContext<'_>) {
		let entity_name = entity_display_name(context.scene, entity);
		let name = cloned_component::<Name>(context.scene, entity).unwrap_or(Name { name: entity_name });
		self.show_name(ui, context, entity, name);
		ui.label(entity_header_metadata(context.scene, entity));
		ui.separator();

		let mut displayed = 0usize;

		if let Some(tag) = cloned_component::<Tag>(context.scene, entity) {
			self.show_tag(ui, context, entity, tag);
			displayed += 1;
		}

		if let Some(transform) = cloned_component::<Transform>(context.scene, entity) {
			self.show_transform(ui, context, entity, transform);
			displayed += 1;
		}

		if context.scene.world().get::<&Parent>(entity).is_ok() {
			show_transform_inheritance(ui, context, entity);
			displayed += 1;
		}

		if context.scene.world().get::<&Inactive>(entity).is_ok() {
			self.show_inactive(ui, context, entity);
			displayed += 1;
		}

		if context.scene.world().get::<&EditorOnly>(entity).is_ok() {
			self.show_editor_only(ui, context, entity);
			displayed += 1;
		}

		if let Some(sprite) = cloned_component::<Sprite>(context.scene, entity) {
			self.show_sprite(ui, context, entity, sprite);
			displayed += 1;
		}

		if let Some(camera) = cloned_component::<Camera>(context.scene, entity) {
			self.show_camera(ui, context, entity, camera);
			displayed += 1;
		}

		if let Some(rigid_body) = cloned_component::<RigidBody>(context.scene, entity) {
			self.show_rigidbody(ui, context, entity, rigid_body);
			displayed += 1;
		}

		if let Some(scripts) = cloned_component::<ZeScripts>(context.scene, entity) {
			// TODO: script list items need stable, collision-proof egui IDs
			// (e.g. keyed by class_path or a persistent script instance id,
			// not by Vec index) to avoid the "first use of widget ID" warning
			// when two scripts with the same initial ID text are added.
			for script in &scripts.list {
				self.show_script(ui, context, entity, script.clone());
				displayed += 1;
			}
		}

		if let Some(collider) = cloned_component::<Collider>(context.scene, entity) {
			self.show_collider(ui, context, entity, collider);
			displayed += 1;
		}

		if let Some(settings) = cloned_component::<PhysicsSettings>(context.scene, entity) {
			self.show_physics_settings(ui, context, entity, settings);
			displayed += 1;
		}

		if let Some(button) = cloned_component::<UIButton>(context.scene, entity) {
			self.show_ui_button(ui, context, entity, button);
			displayed += 1;
		}
		if let Some(bar) = cloned_component::<UIBar>(context.scene, entity) {
			self.show_ui_bar(ui, context, entity, bar);
			displayed += 1;
		}
		if let Some(text) = cloned_component::<UIText>(context.scene, entity) {
			self.show_ui_text(ui, context, entity, text);
			displayed += 1;
		}

		if let Some(audio_source) = cloned_component::<AudioSource>(context.scene, entity) {
			self.show_audio_source(ui, context, entity, audio_source);
			displayed += 1;
		}

		if displayed == 0 {
			ui.label("No inspectable components");
		}
	}

	fn show_transform(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut transform: Transform,
	) {
		let response = show_removable_component(ui, InspectableComponent::Transform.label(), |ui| {
			let mut field_edit = FieldEdit::default();

			ui.horizontal(|ui| {
				ui.label("Position");
				let x = ui.add(drag_value_f32(&mut transform.position.x, 0.05));
				let y = ui.add(drag_value_f32(&mut transform.position.y, 0.05));
				let z = ui.add(drag_value_f32(&mut transform.position.z, 0.05));
				field_edit.include(response_field_edit(&x));
				field_edit.include(response_field_edit(&y));
				field_edit.include(response_field_edit(&z));
			});
			ui.horizontal(|ui| {
				ui.label("Scale");
				let x = ui.add(drag_value_f32(&mut transform.scale.x, 0.05));
				let y = ui.add(drag_value_f32(&mut transform.scale.y, 0.05));
				let z = ui.add(drag_value_f32(&mut transform.scale.z, 0.05));
				field_edit.include(response_field_edit(&x));
				field_edit.include(response_field_edit(&y));
				field_edit.include(response_field_edit(&z));
			});

			let (mut rx, mut ry, mut rz) = transform.rotation.to_euler(ze_core::glam::EulerRot::XYZ);
			let mut rotation_degrees = Vec3::new(rx.to_degrees(), ry.to_degrees(), rz.to_degrees());
			ui.horizontal(|ui| {
				ui.label("Rotation");
				let x = ui.add(drag_value_f32(&mut rotation_degrees.x, 0.05));
				let y = ui.add(drag_value_f32(&mut rotation_degrees.y, 0.05));
				let z = ui.add(drag_value_f32(&mut rotation_degrees.z, 0.05));
				field_edit.include(response_field_edit(&x));
				field_edit.include(response_field_edit(&y));
				field_edit.include(response_field_edit(&z));
			});
			if field_edit.changed {
				rx = rotation_degrees.x.to_radians();
				ry = rotation_degrees.y.to_radians();
				rz = rotation_degrees.z.to_radians();
				transform.rotation = Quat::from_euler(ze_core::glam::EulerRot::XYZ, rx, ry, rz);
			}

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut Transform>(entity) {
					**current = transform;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::Transform);
	}

	fn flush_dirty_text_buffer(&mut self) {
		if let Some(current_path) = self.opened_code_path.clone()
			&& self.code_buffer != self.code_buffer_snapshot
		{
			if let Err(error) = fs::write(&current_path, &self.code_buffer) {
				ze_log::error!("Failed to auto-save file {:?}: {}", current_path, error);
			} else {
				self.code_buffer_snapshot = self.code_buffer.clone();
			}
		}
	}

	fn show_asset(&mut self, ui: &mut Ui, path: &Path, context: &mut EditorPanelContext<'_>) {
		self.flush_dirty_text_buffer();
		ui.heading("Asset");
		ui.label(path.display().to_string());
		ui.separator();

		if path.file_name().and_then(|name| name.to_str()) == Some(PROJECT_FILE_NAME) {
			self.show_project_settings(ui, path, context);
			return;
		}

		let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
		if file_name.ends_with(".zescene.json") {
			self.show_scene_asset(ui);
			return;
		}

		if file_name == ".zeignore" || file_name == ".gitignore" {
			self.show_ignore_file(ui, path);
			return;
		}

		let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");

		match extension {
			"cs" => self.show_csharp_file(ui, path),
			"txt" | "md" | "json" | "xml" | "toml" | "yaml" | "yml" | "wesl" => {
				self.show_text_file(ui, path, extension);
			}
			"png" | "jpg" | "jpeg" | "gif" | "bmp" => {
				self.show_image_asset(ui, path);
			}
			_ => {
				self.show_file_metadata(ui, path);
			}
		}
	}

	#[allow(clippy::unused_self)]
	fn show_scene_asset(&self, ui: &mut Ui) {
		ui.centered_and_justified(|ui| {
			ui.label(
				egui::RichText::new("For editing scenes, use this app, not the inspector :)")
					.size(14.0)
					.color(egui::Color32::from_rgb(130, 140, 160)),
			);
		});
	}

	fn show_text_file(&mut self, ui: &mut Ui, path: &Path, language: &str) {
		let should_reload = self
			.opened_code_path
			.as_deref()
			.is_none_or(|opened_path| opened_path != path);

		if should_reload {
			match fs::read_to_string(path) {
				Ok(source) => {
					self.code_buffer = source;
					self.code_buffer_snapshot = self.code_buffer.clone();
					self.opened_code_path = Some(path.to_path_buf());
				}
				Err(error) => {
					ui.label(format!("Failed to read file: {error}"));
					return;
				}
			}
		}

		let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

		let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
			let mut layout_job =
				egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, text.as_str(), language);

			layout_job.wrap.max_width = wrap_width;

			ui.fonts_mut(|fonts| fonts.layout_job(layout_job))
		};

		egui::ScrollArea::vertical().show(ui, |ui| {
			ui.add(
				TextEdit::multiline(&mut self.code_buffer)
					.font(egui::TextStyle::Monospace)
					.code_editor()
					.desired_width(f32::INFINITY)
					.desired_rows(28)
					.layouter(&mut layouter),
			);
		});
	}

	fn show_image_asset(&mut self, ui: &mut Ui, path: &Path) {
		let metadata = match fs::metadata(path) {
			Ok(m) => m,
			Err(error) => {
				ui.label(format!("Failed to read file metadata: {error}"));
				return;
			}
		};

		let file_size = metadata.len();
		let modified = metadata
			.modified()
			.map_or_else(|_| "unknown".to_string(), Self::format_modified_time);

		let size_str = if file_size < 1024 {
			format!("{file_size} B")
		} else if file_size < 1024 * 1024 {
			format!("{:.1} KB", file_size as f64 / 1024.0)
		} else {
			format!("{:.1} MB", file_size as f64 / (1024.0 * 1024.0))
		};

		ui.label(format!("Size: {size_str}"));
		ui.label(format!("Modified: {modified}"));

		if let Ok(modified_time) = metadata.modified() {
			let needs_reload = self
				.preview_cache
				.as_ref()
				.is_none_or(|(cached_path, cached_time, _)| cached_path != path || *cached_time != modified_time);

			if needs_reload {
				self.preview_cache = None;
				match image::open(path) {
					Ok(img) => {
						let rgba = img.to_rgba8();
						let (w, h) = rgba.dimensions();
						let pixels = rgba.into_raw();
						let color_image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
						let texture =
							ui.ctx()
								.load_texture(path.to_string_lossy(), color_image, TextureOptions::LINEAR);
						self.preview_cache = Some((path.to_path_buf(), modified_time, texture));
					}
					Err(error) => {
						ui.label(format!("Failed to decode image: {error}"));
						return;
					}
				}
			}

			if let Some((_, _, texture)) = &self.preview_cache {
				let max_width = ui.available_width().min(400.0);
				let aspect_ratio = texture.size()[0] as f32 / texture.size()[1] as f32;
				let preview_size = egui::vec2(max_width, max_width / aspect_ratio);
				ui.image(egui::load::SizedTexture::new(texture.id(), preview_size));
			}
		}
	}

	#[allow(clippy::unused_self)]
	fn show_file_metadata(&self, ui: &mut Ui, path: &Path) {
		let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
		let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("none");

		ui.label(format!("Name: {file_name}"));
		ui.label(format!("Extension: .{extension}"));

		match fs::metadata(path) {
			Ok(metadata) => {
				let file_size = metadata.len();
				let size_str = if file_size < 1024 {
					format!("{file_size} B")
				} else if file_size < 1024 * 1024 {
					format!("{:.1} KB", file_size as f64 / 1024.0)
				} else {
					format!("{:.1} MB", file_size as f64 / (1024.0 * 1024.0))
				};
				ui.label(format!("Size: {size_str}"));

				if let Ok(modified) = metadata.modified() {
					ui.label(format!("Modified: {}", Self::format_modified_time(modified)));
				}
			}
			Err(error) => {
				ui.label(format!("Error reading file: {error}"));
			}
		}
	}

	fn show_ignore_file(&mut self, ui: &mut Ui, path: &Path) {
		let should_reload = self
			.opened_code_path
			.as_deref()
			.is_none_or(|opened_path| opened_path != path);

		if should_reload {
			match fs::read_to_string(path) {
				Ok(source) => {
					self.code_buffer = source;
					self.code_buffer_snapshot = self.code_buffer.clone();
					self.opened_code_path = Some(path.to_path_buf());
				}
				Err(error) => {
					ui.label(format!("Failed to read file: {error}"));
					return;
				}
			}
		}

		ui.heading("Ignore Patterns");
		ui.separator();

		let mut file_changed = false;

		egui::ScrollArea::vertical().show(ui, |ui| {
			let lines: Vec<&str> = self.code_buffer.lines().collect();
			let mut remove_indices = Vec::new();

			for (i, line) in lines.iter().enumerate() {
				let trimmed = line.trim();
				if trimmed.is_empty() || trimmed.starts_with('#') {
					continue;
				}
				ui.horizontal(|ui| {
					ui.label(trimmed);
					if ui.button("×").clicked() {
						remove_indices.push(i);
					}
				});
			}

			if !remove_indices.is_empty() {
				let mut new_content = String::new();
				for (i, line) in lines.iter().enumerate() {
					if !remove_indices.contains(&i) {
						new_content.push_str(line);
						new_content.push('\n');
					}
				}
				self.code_buffer = new_content;
				file_changed = true;
			}
		});

		ui.separator();
		ui.horizontal(|ui| {
			ui.label("Add pattern:");
			let response = ui.add(egui::TextEdit::singleline(&mut self.ignore_new_pattern).hint_text("e.g. *.tmp"));
			if ui.button("Add").clicked()
				|| (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
			{
				let pattern = self.ignore_new_pattern.trim().to_string();
				if !pattern.is_empty() {
					if !self.code_buffer.is_empty() && !self.code_buffer.ends_with('\n') {
						self.code_buffer.push('\n');
					}
					self.code_buffer.push_str(&pattern);
					self.code_buffer.push('\n');
					self.ignore_new_pattern.clear();
					file_changed = true;
				}
			}
		});

		if file_changed {
			self.code_buffer_snapshot = self.code_buffer.clone();
			self.opened_code_path = Some(path.to_path_buf());
			if let Err(error) = fs::write(path, &self.code_buffer) {
				ze_log::error!("Failed to save ignore file {:?}: {}", path, error);
			}
		}
	}

	fn format_modified_time(time: std::time::SystemTime) -> String {
		let elapsed = time.elapsed().unwrap_or_default();
		let secs = elapsed.as_secs();
		if secs < 60 {
			format!("{secs}s ago")
		} else if secs < 3600 {
			format!("{}m ago", secs / 60)
		} else if secs < 86400 {
			format!("{}h ago", secs / 3600)
		} else {
			format!("{}d ago", secs / 86400)
		}
	}

	fn show_csharp_file(&mut self, ui: &mut Ui, path: &Path) {
		let should_reload = self
			.opened_code_path
			.as_deref()
			.is_none_or(|opened_path| opened_path != path);

		if should_reload {
			match std::fs::read_to_string(path) {
				Ok(source) => {
					self.code_buffer = source;
					self.code_buffer_snapshot = self.code_buffer.clone();
					self.opened_code_path = Some(path.to_path_buf());
				}
				Err(error) => {
					ui.label(format!("Failed to read file: {error}"));
					return;
				}
			}
		}

		let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());

		let mut layouter = |ui: &Ui, text: &dyn TextBuffer, wrap_width: f32| {
			let mut layout_job =
				egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, text.as_str(), "cs");

			layout_job.wrap.max_width = wrap_width;

			ui.fonts_mut(|fonts| fonts.layout_job(layout_job))
		};

		egui::ScrollArea::vertical().show(ui, |ui| {
			ui.add(
				TextEdit::multiline(&mut self.code_buffer)
					.font(egui::TextStyle::Monospace)
					.code_editor()
					.desired_width(f32::INFINITY)
					.desired_rows(28)
					.layouter(&mut layouter),
			);
		});
	}

	fn show_project_settings(&mut self, ui: &mut Ui, path: &Path, _context: &mut EditorPanelContext<'_>) {
		let current_path = path.to_path_buf();
		let should_reload = self.project_settings.as_ref().is_none_or(|state| state.path != path);

		if should_reload {
			if self.project_settings_error_path.as_ref() == Some(&current_path) {
				if let Some(message) = &self.project_settings_error_message {
					ui.label(message);
				}
				return;
			}

			match Project::load(path) {
				Ok(project) => {
					self.project_settings = Some(ProjectSettingsState::from_project(path.to_path_buf(), project));
					self.project_settings_error_path = None;
					self.project_settings_error_message = None;
				}
				Err(error) => {
					ze_log::error!("failed to load project settings {}: {error:?}", path.display());
					self.project_settings_error_path = Some(current_path);
					self.project_settings_error_message = Some(format!("Failed to load project settings: {error}"));
					if let Some(message) = &self.project_settings_error_message {
						ui.label(message);
					}
					return;
				}
			}
		}

		let Some(state) = self.project_settings.as_mut() else {
			ui.label("No project settings available.");
			return;
		};

		label_value(ui, "Project file", &state.path.display().to_string());
		if let Some(message) = &state.status_message {
			ui.label(message);
		}
		ui.separator();

		ui.collapsing("General", |ui| {
			let mut changed = false;
			changed |= text_field(ui, "Project name", &mut state.project.name);
			changed |= text_field(ui, "Version", &mut state.project.game.version);
			changed |= text_field(ui, "Main scene", &mut state.project.main_scene);

			if changed {
				state.status_message = None;
				state.dirty = true;
			}
		});

		ui.collapsing("Tags", |ui| {
			let mut changed = false;
			let mut remove_idx = None;
			for (i, tag) in state.game_data.tags.iter().enumerate() {
				ui.horizontal(|ui| {
					ui.label(tag);
					if ui.small_button("×").clicked() {
						remove_idx = Some(i);
					}
				});
			}
			if let Some(i) = remove_idx {
				state.game_data.tags.remove(i);
				changed = true;
			}
			ui.horizontal(|ui| {
				let resp = ui.text_edit_singleline(&mut state.new_tag_buffer);
				if (ui.button("Add").clicked() || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
					&& !state.new_tag_buffer.trim().is_empty()
				{
					let trimmed = state.new_tag_buffer.trim().to_string();
					if trimmed.len() <= 64 && !state.game_data.tags.iter().any(|t| t.eq_ignore_ascii_case(&trimmed)) {
						state.game_data.tags.push(trimmed);
						state.new_tag_buffer.clear();
						changed = true;
					}
				}
			});
			if changed {
				state.status_message = None;
				state.dirty = true;
			}
		});

		ui.collapsing("Scenes", |ui| {
			if state.scenes.is_empty() {
				ui.label("No scenes found.");
			} else {
				for scene in &state.scenes {
					if scene == &state.project.main_scene {
						ui.label(format!("{scene} (main)"));
					} else {
						ui.label(scene);
					}
				}
			}
		});

		ui.collapsing("Paths", |ui| {
			let mut changed = false;
			changed |= path_text_field(ui, "Asset dir", &mut state.asset_dir_text);
			changed |= path_text_field(ui, "Scripts DLL", &mut state.scripts_dll_text);
			changed |= path_text_field(ui, "Icon path", &mut state.icon_path_text);
			let icon_was_set = state.project.game.icon_path.is_some();
			if ui.button("Clear icon").clicked() {
				state.project.game.icon_path = None;
				state.icon_path_text.clear();
				changed = true;
			}
			if icon_was_set && state.icon_path_text.trim().is_empty() {
				state.project.game.icon_path = None;
			}

			if let Ok(csproj_path) = state.project.csproj_path() {
				label_value(
					ui,
					"Scripts project path",
					&display_project_relative_path(&state.project.root_dir, &csproj_path),
				);
			}

			if changed {
				state.status_message = None;
				state.dirty = true;
			}
		});

		ui.collapsing("Physics", |ui| {
			let mut changed = false;
			changed |= vec3_editor(ui, "Gravity", &mut state.project.physics.gravity).changed;
			changed |= drag_f32(ui, "Timestep", &mut state.project.physics.timestep).changed;
			changed |= drag_u32(ui, "Solver iterations", &mut state.project.physics.solver_iterations);
			if changed {
				state.status_message = None;
				state.dirty = true;
			}
		});
	}
}

impl ProjectSettingsState {
	fn from_project(path: PathBuf, project: Project) -> Self {
		let asset_dir_text = display_project_relative_path(&project.root_dir, &project.asset_dir);
		let scripts_dll_text = display_project_relative_path(&project.root_dir, &project.scripts_dll);
		let icon_path_text = project
			.game
			.icon_path
			.as_ref()
			.map(|icon_path| display_project_relative_path(&project.root_dir, icon_path))
			.unwrap_or_default();
		let scenes = project.scenes.clone();

		let game_data = GameData::load_or_create(project.game_data_path());

		Self {
			path,
			project,
			asset_dir_text,
			scripts_dll_text,
			icon_path_text,
			scenes,
			game_data,
			status_message: None,
			dirty: false,
			new_tag_buffer: String::new(),
		}
	}
}

fn text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool {
	let mut changed = false;
	ui.horizontal(|ui| {
		ui.label(label);
		changed = ui.text_edit_singleline(value).changed();
	});
	changed
}

fn label_value(ui: &mut Ui, label: &str, value: &str) {
	ui.horizontal(|ui| {
		ui.add_sized([120.0, 0.0], egui::Label::new(label));
		ui.monospace(value);
	});
}

fn path_text_field(ui: &mut Ui, label: &str, value: &mut String) -> bool { text_field(ui, label, value) }

fn drag_u32(ui: &mut Ui, label: &str, value: &mut u32) -> bool {
	let mut changed = false;
	ui.horizontal(|ui| {
		ui.label(label);
		changed = ui.add(drag_value_u32(value, 1.0)).changed();
	});
	changed
}

fn display_project_relative_path(root: &Path, path: &Path) -> String {
	path.strip_prefix(root).map_or_else(
		|_| path.display().to_string(),
		|relative| relative.display().to_string(),
	)
}

fn save_project_settings_state(state: &mut ProjectSettingsState) -> Result<Project, String> {
	let mut project = state.project.clone();
	project.game.name.clone_from(&project.name);
	project.asset_dir = resolve_project_path_required(&project.root_dir, &state.asset_dir_text, "asset directory")?;
	project.scripts_dll = resolve_project_path_required(&project.root_dir, &state.scripts_dll_text, "scripts DLL")?;
	project.game.icon_path = if state.icon_path_text.trim().is_empty() {
		None
	} else {
		Some(resolve_project_path_required(
			&project.root_dir,
			&state.icon_path_text,
			"icon path",
		)?)
	};
	let main_scene = project.main_scene.clone();
	project
		.set_main_scene(main_scene)
		.map_err(|error| format!("main scene: {error}"))?;
	project
		.validate_structure()
		.map_err(|error| format!("project settings are invalid: {error}"))?;

	project
		.save(&state.path)
		.map_err(|error| format!("failed to save project settings {}: {error}", state.path.display()))?;

	let game_data_path = project.game_data_path();
	state
		.game_data
		.save(&game_data_path)
		.map_err(|error| format!("failed to save GameData.toml: {error}"))?;

	state.project = project.clone();
	state.asset_dir_text = display_project_relative_path(&project.root_dir, &project.asset_dir);
	state.scripts_dll_text = display_project_relative_path(&project.root_dir, &project.scripts_dll);
	state.icon_path_text = project
		.game
		.icon_path
		.as_ref()
		.map(|icon_path| display_project_relative_path(&project.root_dir, icon_path))
		.unwrap_or_default();
	state.scenes.clone_from(&project.scenes);
	state.status_message = Some("Saved ZEProject.toml".to_string());
	state.dirty = false;

	Ok(project)
}

fn resolve_project_path_required(base_dir: &Path, text: &str, label: &str) -> Result<PathBuf, String> {
	let text = text.trim();
	if text.is_empty() {
		return Err(format!("{label} cannot be empty"));
	}

	resolve_project_path(base_dir, Path::new(text)).map_err(|error| format!("{label}: {error}"))
}

fn resolve_project_path(base_dir: &Path, path: &Path) -> Result<PathBuf, String> {
	if path
		.components()
		.any(|component| matches!(component, std::path::Component::ParentDir))
	{
		return Err(format!(
			"path `{}` must not traverse outside the project root",
			path.display()
		));
	}

	if path.is_absolute() {
		if path.starts_with(base_dir) {
			return Ok(path.to_path_buf());
		}

		return Err(format!(
			"path `{}` is outside project root `{}`",
			path.display(),
			base_dir.display()
		));
	}

	Ok(base_dir.join(path))
}

fn cloned_component<T>(scene: &ze_ecs::Scene, entity: EntityId) -> Option<T>
where
	T: ze_ecs::Component + Clone + 'static,
{
	scene.world().get::<&T>(entity).ok().map(|component| component.clone())
}

fn entity_display_name(scene: &ze_ecs::Scene, entity: EntityId) -> String {
	scene
		.world()
		.get::<&Name>(entity)
		.map_or_else(|_| format!("Entity {}", entity.index()), |name| name.name.clone())
}

fn entity_header_metadata(scene: &ze_ecs::Scene, entity: EntityId) -> String {
	let tag = scene
		.world()
		.get::<&Tag>(entity)
		.map_or_else(|_| "No tag".to_string(), |tag| format!("Tag: {}", tag.tag));
	format!("{tag} | ID {}", entity.index())
}

fn available_components(scene: &ze_ecs::Scene, entity: EntityId) -> Vec<InspectableComponent> {
	InspectableComponent::ADDABLE
		.into_iter()
		.filter(|component| *component != InspectableComponent::Transform || !component.is_present(scene, entity))
		.collect()
}

fn label_matches_search(label: &str, query: &str) -> bool { query.is_empty() || label.to_lowercase().contains(query) }

fn default_sprite() -> Sprite {
	Sprite {
		texture: TextureSource::File(AssetRef::game("")),
		size: SpriteSize::Auto,
		color: SpriteColorSettings::default(),
		settings: SpriteSettings::default(),
		glow_strength: 0.0,
	}
}

const fn default_camera() -> Camera {
	Camera {
		projection: CameraProjection::Orthographic {
			size: 10.0,
			near: -100.0,
			far: 100.0,
		},
		primary: false,
		clear_color: [0.12, 0.12, 0.13, 1.0],
	}
}

fn add_component_to_entity(context: &mut EditorPanelContext<'_>, entity: EntityId, component: InspectableComponent) {
	if component == InspectableComponent::Transform && component.is_present(context.scene, entity) {
		return;
	}

	record_scene_edit(context, |scene| {
		component.add_to(scene, entity);
	});
}

fn add_script_to_entity(context: &mut EditorPanelContext<'_>, entity: EntityId, class_path: String) {
	record_scene_edit(context, |scene| {
		let mut scripts = cloned_component::<ZeScripts>(scene, entity).unwrap_or(ZeScripts { list: Vec::new() });
		scripts.list.push(ScriptData {
			path: class_path,
			enabled: true,
			fields: std::collections::BTreeMap::new(),
		});
		scene.entity_mut(entity).add_component(scripts);
	});
}

fn remove_component_from_entity(
	context: &mut EditorPanelContext<'_>,
	entity: EntityId,
	component: InspectableComponent,
) {
	if !component.is_present(context.scene, entity) {
		return;
	}

	record_scene_edit(context, |scene| {
		component.remove_from(scene, entity);
	});
}

impl InspectorPanel {
	fn show_name(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId, mut name: Name) {
		let response = ui.add(
			TextEdit::singleline(&mut name.name)
				.font(egui::TextStyle::Heading)
				.desired_width(f32::INFINITY),
		);
		let field_edit = response_field_edit(&response);
		self.apply_field_edit(context, field_edit, |scene| {
			if let Ok(mut current) = scene.world_mut().get::<&mut Name>(entity) {
				**current = name;
			} else {
				scene.entity_mut(entity).add_component(name);
			}
		});
	}

	fn show_inspector_context_menu(
		&mut self,
		response: &egui::Response,
		entity: EntityId,
		context: &mut EditorPanelContext<'_>,
	) {
		let mut action = InspectorContextMenuAction::default();
		response.context_menu(|ui| {
			self.show_add_component_menu(ui, entity, context, &mut action);
		});
		self.apply_context_menu_action(context, entity, action, None);
	}

	fn show_component_context_menu(
		&mut self,
		response: &egui::Response,
		entity: EntityId,
		context: &mut EditorPanelContext<'_>,
		component: InspectableComponent,
	) {
		let mut action = InspectorContextMenuAction::default();
		response.context_menu(|ui| {
			self.show_add_component_menu(ui, entity, context, &mut action);
			if ui.button("Remove Component").clicked() {
				action.remove_component = true;
				ui.close();
			}
		});
		self.apply_context_menu_action(context, entity, action, Some(component));
	}

	fn show_add_component_menu(
		&mut self,
		ui: &mut Ui,
		entity: EntityId,
		context: &mut EditorPanelContext<'_>,
		action: &mut InspectorContextMenuAction,
	) {
		let available = available_components(context.scene, entity);

		if self.script_classes_cache.is_none() {
			self.script_classes_cache = script_classes(context).ok();
		}

		let close_behavior = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
		SubMenuButton::new("Add Component").config(close_behavior).ui(ui, |ui| {
			ui.set_min_width(240.0);
			let search_response = ui.add(
				TextEdit::singleline(&mut self.component_search)
					.hint_text("Search components")
					.desired_width(f32::INFINITY),
			);
			search_response.request_focus();
			ui.separator();

			let query = self.component_search.trim().to_lowercase();
			let mut shown = false;
			for component in available.iter().copied() {
				if !label_matches_search(component.label(), &query) {
					continue;
				}

				shown = true;
				if ui.button(component.label()).clicked() {
					action.add_component = Some(component);
					ui.close();
				}
			}

			match &self.script_classes_cache {
				Some(classes) => {
					for class_path in classes {
						let label = format!("Script: {class_path}");
						if !label_matches_search(&label, &query) {
							continue;
						}

						shown = true;
						if ui.button(label).clicked() {
							action.add_script = Some(class_path.clone());
							ui.close();
						}
					}
				}
				None => {
					ui.colored_label(egui::Color32::YELLOW, "C# script classes are unavailable.");
				}
			}

			if !shown {
				ui.label("No matching components.");
			}
		});
	}

	fn apply_context_menu_action(
		&mut self,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		action: InspectorContextMenuAction,
		removable_component: Option<InspectableComponent>,
	) {
		if let Some(component) = action.add_component {
			self.component_search.clear();
			add_component_to_entity(context, entity, component);
		}
		if let Some(class_path) = action.add_script {
			self.component_search.clear();
			add_script_to_entity(context, entity, class_path);
		}
		if action.remove_component
			&& let Some(component) = removable_component
		{
			remove_component_from_entity(context, entity, component);
		}
	}

	fn show_tag(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId, mut tag: Tag) {
		let mut field_edit = FieldEdit::default();

		let response = show_removable_component(ui, InspectableComponent::Tag.label(), |ui| {
			ui.horizontal(|ui| {
				ui.label("Tag");

				let tag_list: Vec<String> = self.resolve_tag_list(context);

				let current_tag = if tag.tag.is_empty() {
					"Untagged".to_string()
				} else {
					tag.tag.clone()
				};

				let mut selected = current_tag.clone();

				egui::ComboBox::from_id_salt("entity_tag")
					.selected_text(&selected)
					.show_ui(ui, |ui| {
						for t in &tag_list {
							ui.selectable_value(&mut selected, t.clone(), t.as_str());
						}
					});

				if selected != current_tag {
					tag.tag = selected;
					field_edit.include(FieldEdit::changed(true));
				}

				let add_resp = ui.button("+");
				if add_resp.clicked() {
					self.show_add_tag_popup = true;
				}

				egui::Popup::from_response(&add_resp)
					.open(self.show_add_tag_popup)
					.close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
					.id(egui::Id::new("zero_editor_tag_popup"))
					.show(|ui| {
						ui.horizontal(|ui| {
							ui.label("New Tag:");
							let resp =
								ui.add(egui::TextEdit::singleline(&mut self.new_tag_buffer).desired_width(120.0));
							resp.request_focus();

							if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
								self.add_new_tag(context);
							}

							if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
								self.new_tag_buffer.clear();
								self.show_add_tag_popup = false;
							}

							if resp.clicked_elsewhere() && !resp.gained_focus() {
								self.new_tag_buffer.clear();
								self.show_add_tag_popup = false;
							}
						});
					});
			});
		});
		self.apply_field_edit(context, field_edit, |scene| {
			if let Ok(mut current) = scene.world_mut().get::<&mut Tag>(entity) {
				**current = tag;
			}
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::Tag);
	}

	fn resolve_tag_list(&self, context: &EditorPanelContext<'_>) -> Vec<String> {
		if let Some(state) = &self.project_settings {
			if state.game_data.tags.is_empty() {
				return Self::FALLBACK_TAGS.iter().map(|&s| s.to_string()).collect();
			}
			return state.game_data.tags.clone();
		}
		let game_data = context
			.project
			.map(|project| ze_project::GameData::load_or_create(project.game_data_path()));
		if let Some(ref gd) = game_data {
			if gd.tags.is_empty() {
				return Self::FALLBACK_TAGS.iter().map(|&s| s.to_string()).collect();
			}
			return gd.tags.clone();
		}
		Self::FALLBACK_TAGS.iter().map(|&s| s.to_string()).collect()
	}

	fn add_new_tag(&mut self, context: &EditorPanelContext<'_>) {
		let trimmed = self.new_tag_buffer.trim().to_string();
		if trimmed.is_empty() {
			return;
		}
		let game_data_path = context.project.map(ze_project::Project::game_data_path);

		if let Some(state) = &mut self.project_settings {
			if state.game_data.tags.iter().any(|t| t.eq_ignore_ascii_case(&trimmed)) {
				self.new_tag_buffer.clear();
				return;
			}
			state.game_data.tags.push(trimmed);
			if let Some(ref path) = game_data_path {
				let _ = state.game_data.save(path);
			}
		} else if let Some(ref path) = game_data_path {
			let mut gd = ze_project::GameData::load_or_create(path);
			if gd.tags.iter().any(|t| t.eq_ignore_ascii_case(&trimmed)) {
				self.new_tag_buffer.clear();
				return;
			}
			gd.tags.push(trimmed);
			let _ = gd.save(path);
		}

		self.new_tag_buffer.clear();
		self.show_add_tag_popup = false;
	}

	fn show_inactive(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId) {
		let response = show_removable_component(ui, InspectableComponent::Inactive.label(), |ui| {
			ui.monospace("true");
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::Inactive);
	}

	fn show_editor_only(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId) {
		let response = show_removable_component(ui, InspectableComponent::EditorOnly.label(), |ui| {
			ui.monospace("true");
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::EditorOnly);
	}
}

impl InspectorPanel {
	fn show_sprite(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId, mut sprite: Sprite) {
		let response = show_removable_component(ui, InspectableComponent::Sprite.label(), |ui| {
			let mut field_edit = FieldEdit::default();

			match sprite.texture.clone() {
				TextureSource::File(asset_ref) => {
					let mut texture_path = asset_ref.path.clone();
					ui.horizontal(|ui| {
						ui.label("Texture");
						let response = ui.text_edit_singleline(&mut texture_path);
						field_edit.include(response_field_edit(&response));
					});
					if field_edit.changed {
						sprite.texture = TextureSource::File(AssetRef::game(texture_path));
					}
				}
				TextureSource::SheetCell { sheet_path, cell_id } => {
					ui.horizontal(|ui| {
						ui.label("Texture");
						ui.label(format!("Sheet cell: {sheet_path} #{}", cell_id.0));
					});
				}
			}

			ui.horizontal(|ui| {
				ui.label("Size");
				match &mut sprite.size {
					SpriteSize::Auto => {
						ui.label("Auto");
						let custom_clicked = ui.button("Custom").clicked();
						field_edit.include(FieldEdit::changed(custom_clicked));
						if custom_clicked {
							sprite.size = SpriteSize::Custom {
								width: 1.0,
								height: 1.0,
							};
						}
					}
					SpriteSize::Custom { width, height } => {
						ui.label("W");
						let width_response = ui.add(drag_value_f32(width, 0.05));
						field_edit.include(response_field_edit(&width_response));
						ui.label("H");
						let height_response = ui.add(drag_value_f32(height, 0.05));
						field_edit.include(response_field_edit(&height_response));
						let auto_clicked = ui.button("Auto").clicked();
						field_edit.include(FieldEdit::changed(auto_clicked));
						if auto_clicked {
							sprite.size = SpriteSize::Auto;
						}
					}
				}
			});

			ui.horizontal(|ui| {
				ui.label("Visible");
				field_edit.include(response_field_edit(&ui.checkbox(&mut sprite.settings.visible, "")));
				ui.label("Flip X");
				field_edit.include(response_field_edit(&ui.checkbox(&mut sprite.settings.flip_x, "")));
				ui.label("Flip Y");
				field_edit.include(response_field_edit(&ui.checkbox(&mut sprite.settings.flip_y, "")));
			});
			field_edit.include(drag_i32(ui, "Layer", &mut sprite.settings.layer));
			field_edit.include(drag_f32(
				ui,
				"Texture rotation",
				&mut sprite.settings.texture_rotation_degrees,
			));

			let mut tint = sprite.color.tint.unwrap_or([1.0, 1.0, 1.0, 1.0]);
			let mut use_tint = sprite.color.tint.is_some();
			field_edit.include(response_field_edit(&ui.checkbox(&mut use_tint, "Use tint")));
			ui.horizontal(|ui| {
				ui.label("Tint");
				if use_tint {
					let response = ui.color_edit_button_rgba_unmultiplied(&mut tint);
					field_edit.include(response_field_edit(&response));
				} else {
					ui.label("None");
				}
			});
			sprite.color.tint = use_tint.then_some(tint);

			egui::ComboBox::from_label("Color mode")
				.selected_text(sprite_color_mode_label(sprite.color.mode))
				.show_ui(ui, |ui| {
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut sprite.color.mode,
						SpriteColorMode::None,
						"None",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut sprite.color.mode,
						SpriteColorMode::Multiply,
						"Multiply",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut sprite.color.mode,
						SpriteColorMode::GrayscaleTint,
						"Grayscale Tint",
					)));
				});
			field_edit.include(drag_f32(ui, "Color strength", &mut sprite.color.strength));
			field_edit.include(drag_f32(
				ui,
				"Saturation threshold",
				&mut sprite.color.saturation_threshold,
			));

			ui.horizontal(|ui| {
				ui.label("Glow strength");
				let response = ui.add(
					egui::DragValue::new(&mut sprite.glow_strength)
						.speed(0.05)
						.range(0.0..=200.0),
				);
				field_edit.include(response_field_edit(&response));
			});

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut Sprite>(entity) {
					**current = sprite;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::Sprite);
	}
}

impl InspectorPanel {
	fn show_camera(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId, mut camera: Camera) {
		let response = show_removable_component(ui, InspectableComponent::Camera.label(), |ui| {
			let mut field_edit = FieldEdit::default();
			let response = ui.checkbox(&mut camera.primary, "Primary");
			field_edit.include(response_field_edit(&response));

			let mut clear_color = camera.clear_color;
			ui.horizontal(|ui| {
				ui.label("Clear color");
				let response = ui.color_edit_button_rgba_unmultiplied(&mut clear_color);
				field_edit.include(response_field_edit(&response));
			});
			camera.clear_color = clear_color;

			let mut projection_kind = match camera.projection {
				CameraProjection::Orthographic { .. } => CameraProjectionKind::Orthographic,
				CameraProjection::Perspective { .. } => CameraProjectionKind::Perspective,
			};
			egui::ComboBox::from_label("Projection")
				.selected_text(projection_kind.label())
				.show_ui(ui, |ui| {
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut projection_kind,
						CameraProjectionKind::Orthographic,
						"Orthographic",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut projection_kind,
						CameraProjectionKind::Perspective,
						"Perspective",
					)));
				});
			camera.projection = match (projection_kind, camera.projection) {
				(
					CameraProjectionKind::Orthographic,
					CameraProjection::Orthographic {
						mut size,
						mut near,
						mut far,
					},
				) => {
					field_edit.include(drag_f32(ui, "Size", &mut size));
					field_edit.include(drag_f32(ui, "Near", &mut near));
					field_edit.include(drag_f32(ui, "Far", &mut far));
					CameraProjection::Orthographic { size, near, far }
				}
				(
					CameraProjectionKind::Perspective,
					CameraProjection::Perspective {
						fov_y_radians,
						mut near,
						mut far,
					},
				) => {
					let mut fov_degrees = fov_y_radians.to_degrees();
					field_edit.include(drag_f32(ui, "FOV", &mut fov_degrees));
					field_edit.include(drag_f32(ui, "Near", &mut near));
					field_edit.include(drag_f32(ui, "Far", &mut far));
					CameraProjection::Perspective {
						fov_y_radians: fov_degrees.to_radians(),
						near,
						far,
					}
				}
				(CameraProjectionKind::Orthographic, CameraProjection::Perspective { near, far, .. }) => {
					CameraProjection::Orthographic { size: 10.0, near, far }
				}
				(CameraProjectionKind::Perspective, CameraProjection::Orthographic { near, far, .. }) => {
					CameraProjection::Perspective {
						fov_y_radians: 60.0_f32.to_radians(),
						near,
						far,
					}
				}
			};

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut Camera>(entity) {
					**current = camera;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::Camera);
	}
}

impl InspectorPanel {
	fn show_rigidbody(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut rigid_body: RigidBody,
	) {
		let response = show_removable_component(ui, InspectableComponent::RigidBody.label(), |ui| {
			let mut field_edit = FieldEdit::default();
			egui::ComboBox::from_label("Body type")
				.selected_text(rigidbody_type_label(rigid_body.body_type))
				.show_ui(ui, |ui| {
					field_edit.include(rigidbody_type_option(
						ui,
						&mut rigid_body.body_type,
						RigidBodyType::Static,
					));
					field_edit.include(rigidbody_type_option(
						ui,
						&mut rigid_body.body_type,
						RigidBodyType::Dynamic,
					));
					field_edit.include(rigidbody_type_option(
						ui,
						&mut rigid_body.body_type,
						RigidBodyType::KinematicPositionBased,
					));
					field_edit.include(rigidbody_type_option(
						ui,
						&mut rigid_body.body_type,
						RigidBodyType::KinematicVelocityBased,
					));
				});
			let response = ui.checkbox(&mut rigid_body.use_gravity, "Use gravity");
			field_edit.include(response_field_edit(&response));
			field_edit.include(drag_f32(ui, "Gravity scale", &mut rigid_body.gravity_scale));
			field_edit.include(drag_f32(ui, "Linear damping", &mut rigid_body.linear_damping));
			field_edit.include(drag_f32(ui, "Angular damping", &mut rigid_body.angular_damping));

			let mut has_mass = rigid_body.mass.is_some();
			let response = ui.checkbox(&mut has_mass, "Override mass");
			let mass_override_edit = response_field_edit(&response);
			field_edit.include(mass_override_edit);
			if mass_override_edit.changed {
				rigid_body.mass = has_mass.then_some(rigid_body.mass.unwrap_or(1.0));
			}
			if let Some(mass) = rigid_body.mass.as_mut() {
				field_edit.include(drag_f32(ui, "Mass", mass));
			}

			ui.label("Freeze");
			ui.horizontal(|ui| {
				field_edit.include(response_field_edit(
					&ui.checkbox(&mut rigid_body.freeze_position_x, "Position X"),
				));
				field_edit.include(response_field_edit(
					&ui.checkbox(&mut rigid_body.freeze_position_y, "Position Y"),
				));
			});
			ui.horizontal(|ui| {
				field_edit.include(response_field_edit(
					&ui.checkbox(&mut rigid_body.freeze_rotation_x, "Rotation X"),
				));
				field_edit.include(response_field_edit(
					&ui.checkbox(&mut rigid_body.freeze_rotation_y, "Rotation Y"),
				));
				field_edit.include(response_field_edit(
					&ui.checkbox(&mut rigid_body.freeze_rotation_z, "Rotation Z"),
				));
			});

			egui::ComboBox::from_label("Collision detection")
				.selected_text(collision_detection_label(rigid_body.collision_detection))
				.show_ui(ui, |ui| {
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut rigid_body.collision_detection,
						CollisionDetection::Discrete,
						"Discrete",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut rigid_body.collision_detection,
						CollisionDetection::Continuous,
						"Continuous",
					)));
				});

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut RigidBody>(entity) {
					**current = rigid_body;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::RigidBody);
	}

	fn show_audio_source(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut audio_source: AudioSource,
	) {
		let response = show_removable_component(ui, InspectableComponent::AudioSource.label(), |ui| {
			let mut field_edit = FieldEdit::default();

			ui.horizontal(|ui| {
				ui.label("Clip Path");
				let response = ui.text_edit_singleline(&mut audio_source.clip_path);
				field_edit.include(response_field_edit(&response));
			});

			ui.horizontal(|ui| {
				ui.label("Volume");
				let response = ui.add(
					egui::DragValue::new(&mut audio_source.volume)
						.speed(0.01)
						.range(0.0..=1.0),
				);
				field_edit.include(response_field_edit(&response));
			});

			field_edit.include(response_field_edit(&ui.checkbox(&mut audio_source.looping, "Looping")));
			field_edit.include(response_field_edit(
				&ui.checkbox(&mut audio_source.play_on_start, "Play On Start"),
			));

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut AudioSource>(entity) {
					**current = audio_source;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::AudioSource);
	}
}

impl InspectorPanel {
	fn show_collider(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut collider: Collider,
	) {
		let response = show_removable_component(ui, InspectableComponent::Collider.label(), |ui| {
			let mut field_edit = FieldEdit::default();
			let mut shape_kind = ColliderShapeKind::from_shape(&collider.shape);
			egui::ComboBox::from_label("Shape")
				.selected_text(shape_kind.label())
				.show_ui(ui, |ui| {
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut shape_kind,
						ColliderShapeKind::Box,
						"Box",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut shape_kind,
						ColliderShapeKind::Circle,
						"Circle",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut shape_kind,
						ColliderShapeKind::Capsule,
						"Capsule",
					)));
					field_edit.include(response_field_edit(&ui.selectable_value(
						&mut shape_kind,
						ColliderShapeKind::ConvexPolygon,
						"Convex Polygon",
					)));
				});

			if shape_kind != ColliderShapeKind::from_shape(&collider.shape) {
				collider.shape = shape_kind.default_shape();
			}

			match &mut collider.shape {
				ColliderShape::Box { half_extents } => {
					let mut size = *half_extents * 2.0;
					let size_edit = vec2_editor(ui, "Box size", &mut size);
					field_edit.include(size_edit);
					if size_edit.changed {
						half_extents.x = size.x.max(0.0) * 0.5;
						half_extents.y = size.y.max(0.0) * 0.5;
					}
				}
				ColliderShape::Circle { radius } => {
					let radius_edit = drag_f32(ui, "Radius", radius);
					field_edit.include(radius_edit);
					if radius_edit.changed {
						*radius = (*radius).max(0.0);
					}
				}
				ColliderShape::Capsule { half_height, radius } => {
					let half_height_edit = drag_f32(ui, "Half height", half_height);
					field_edit.include(half_height_edit);
					if half_height_edit.changed {
						*half_height = (*half_height).max(0.0);
					}
					let radius_edit = drag_f32(ui, "Radius", radius);
					field_edit.include(radius_edit);
					if radius_edit.changed {
						*radius = (*radius).max(0.0);
					}
				}
				ColliderShape::ConvexPolygon { points } => {
					for (index, point) in points.iter_mut().enumerate() {
						field_edit.include(vec2_editor(ui, &format!("Point {index}"), point));
					}
					if points.is_empty() {
						ui.label("No polygon points");
					}
				}
			}

			field_edit.include(drag_f32(ui, "Friction", &mut collider.friction));
			field_edit.include(drag_f32(ui, "Restitution", &mut collider.restitution));
			field_edit.include(drag_f32(ui, "Density", &mut collider.density));
			field_edit.include(response_field_edit(&ui.checkbox(&mut collider.is_sensor, "Sensor")));

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut Collider>(entity) {
					**current = collider;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::Collider);
	}
}

impl InspectorPanel {
	fn show_physics_settings(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut settings: PhysicsSettings,
	) {
		let response = show_removable_component(ui, InspectableComponent::PhysicsSettings.label(), |ui| {
			let mut field_edit = FieldEdit::default();
			field_edit.include(vec2_editor(ui, "Gravity", &mut settings.gravity));
			field_edit.include(response_field_edit(
				&ui.checkbox(&mut settings.enable_debug_draw, "Enable debug draw"),
			));
			field_edit.include(drag_f32(ui, "Physics timestep", &mut settings.physics_timestep));

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut PhysicsSettings>(entity) {
					**current = settings;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::PhysicsSettings);
	}
}

impl InspectorPanel {
	fn show_script(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut script: ScriptData,
	) {
		let original_path = script.path.clone();
		let header = script_component_header(&script.path);
		let response = show_removable_component(ui, &header, |ui| {
			let mut field_edit = FieldEdit::default();
			ui.horizontal(|ui| {
				ui.label("Class");
				let response = ui.text_edit_singleline(&mut script.path);
				field_edit.include(response_field_edit(&response));
			});
			field_edit.include(response_field_edit(&ui.checkbox(&mut script.enabled, "Enabled")));

			field_edit.include(show_script_fields(ui, context, &script.path, &mut script.fields));

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut scripts) = scene.world_mut().get::<&mut ZeScripts>(entity)
					&& let Some(current) = scripts.list.iter_mut().find(|s| s.path == original_path)
				{
					*current = script;
				}
			});
		});
		// Use a dedicated context menu with inline script removal instead of
		// the shared show_component_context_menu, so removing one script does
		// not affect other scripts on the same entity.
		let mut action = InspectorContextMenuAction::default();
		response.context_menu(|ui| {
			self.show_add_component_menu(ui, entity, context, &mut action);
			if ui.button("Remove Script").clicked() {
				action.remove_script_path = Some(original_path);
				ui.close();
			}
		});
		if let Some(script_path) = action.remove_script_path {
			record_scene_edit(context, |scene| {
				let list_empty = scene
					.world_mut()
					.get::<&mut ZeScripts>(entity)
					.is_ok_and(|mut scripts| {
						scripts.list.retain(|s| s.path != script_path);
						scripts.list.is_empty()
					});
				if list_empty {
					let _ = scene.entity_mut(entity).remove_component::<ZeScripts>();
				}
			});
		}
	}
}

fn component_header(label: &str) -> String { format!(" {label}") }

fn show_removable_component(ui: &mut Ui, label: &str, body: impl FnOnce(&mut Ui)) -> egui::Response {
	ui.collapsing(component_header(label), body)
		.header_response
		.on_hover_text("Right-click for component options")
}

fn script_component_header(class_path: &str) -> String {
	let class_path = class_path.trim();
	if class_path.is_empty() {
		"Script".to_string()
	} else {
		class_path.to_string()
	}
}

fn show_transform_inheritance(ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId) {
	let mut inheritance = cloned_component::<TransformInheritance>(context.scene, entity).unwrap_or_default();
	ui.collapsing(component_header("Inheritance"), |ui| {
		ui.label("Parent transform inheritance");
		let mut field_edit = FieldEdit::default();
		field_edit.include(response_field_edit(
			&ui.checkbox(&mut inheritance.inherit_position, "Inherit Position"),
		));
		field_edit.include(response_field_edit(
			&ui.checkbox(&mut inheritance.inherit_rotation, "Inherit Rotation"),
		));
		field_edit.include(response_field_edit(
			&ui.checkbox(&mut inheritance.inherit_scale, "Inherit Scale"),
		));

		if field_edit.changed {
			record_scene_edit(context, |scene| {
				scene.world_mut().add_component(entity, (inheritance,));
			});
		}
	});
}

fn show_script_fields(
	ui: &mut Ui,
	context: &mut EditorPanelContext<'_>,
	class_path: &str,
	fields: &mut std::collections::BTreeMap<String, ScriptFieldValue>,
) -> FieldEdit {
	let mut field_edit = FieldEdit::default();
	ui.separator();
	ui.label("Script Fields");

	let Some(runtime) = context
		.scene
		.with_system_mut::<ScriptingSystem, _>(|scripting, _scene| scripting.runtime())
	else {
		ui.colored_label(egui::Color32::YELLOW, "Script field metadata is unavailable.");
		return FieldEdit::default();
	};

	match runtime.describe_script_fields(class_path) {
		Ok(Some(script_fields)) => {
			if script_fields.is_empty() {
				ui.label("No exposed script fields.");
				return FieldEdit::default();
			}

			for field in script_fields {
				field_edit.include(show_script_field(ui, &field, fields));
			}
		}
		Ok(None) => {
			ui.colored_label(egui::Color32::YELLOW, "Script field metadata is unavailable.");
		}
		Err(error) => {
			ui.colored_label(egui::Color32::YELLOW, "Failed to load script field metadata.");
			ze_log::warn!("failed to load script field metadata for `{class_path}`: {error:?}");
		}
	}

	field_edit
}

fn script_classes(context: &mut EditorPanelContext<'_>) -> Result<Vec<String>, String> {
	let runtime = context
		.scene
		.with_system_mut::<ScriptingSystem, _>(|scripting, _scene| scripting.runtime())
		.ok_or_else(|| "C# scripting runtime is unavailable.".to_string())?;

	let classes = runtime
		.list_script_classes()
		.map_err(|error| format!("Failed to list C# script classes: {error}"))?;

	// Filter out scripts whose field metadata cannot be loaded, which
	// indicates the class is not actually usable (e.g. leftover stubs,
	// mismatched assemblies, or types that fail ResolveScriptType).
	let valid: Vec<String> = classes
		.into_iter()
		.filter(|class_path| match runtime.describe_script_fields(class_path) {
			Ok(Some(_)) => true,
			Ok(None) => {
				ze_log::warn!("script class `{class_path}` resolved but returned no field metadata; listing it anyway");
				true
			}
			Err(error) => {
				ze_log::warn!(
					"script class `{class_path}` field metadata failed to load (will still be listed): {error:?}"
				);
				true
			}
		})
		.collect();

	Ok(valid)
}

fn show_script_field(
	ui: &mut Ui,
	field: &ScriptFieldMetadata,
	fields: &mut std::collections::BTreeMap<String, ScriptFieldValue>,
) -> FieldEdit {
	let Some(kind) = script_field_kind(&field.ty) else {
		show_script_field_readonly(ui, field, script_field_display_value(field, fields));
		return FieldEdit::default();
	};

	let current_value = fields.get(&field.name).cloned();
	let mut field_edit = FieldEdit::default();

	match kind {
		ScriptFieldKind::Bool => {
			let mut value = match script_field_current_value(field, fields, kind) {
				Some(ScriptFieldValue::Bool(value)) => value,
				_ => false,
			};
			ui.horizontal(|ui| {
				ui.monospace(&field.name);
				ui.label(":");
				ui.monospace(&field.ty);
				let response = ui.checkbox(&mut value, "");
				let edit = response_field_edit(&response);
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::Bool(value));
				}
			});
		}
		ScriptFieldKind::Int => {
			let mut value = match script_field_current_value(field, fields, kind) {
				Some(ScriptFieldValue::Int(value)) => value,
				_ => 0,
			};
			ui.horizontal(|ui| {
				ui.monospace(&field.name);
				ui.label(":");
				ui.monospace(&field.ty);
				let response = ui.add(drag_value_i32(&mut value, 1.0));
				let edit = response_field_edit(&response);
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::Int(value));
				}
			});
		}
		ScriptFieldKind::UInt => {
			let mut value = match script_field_current_value(field, fields, kind) {
				Some(ScriptFieldValue::UInt(value)) => value,
				_ => 0,
			};
			ui.horizontal(|ui| {
				ui.monospace(&field.name);
				ui.label(":");
				ui.monospace(&field.ty);
				let response = ui.add(drag_value_u32(&mut value, 1.0));
				let edit = response_field_edit(&response);
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::UInt(value));
				}
			});
		}
		ScriptFieldKind::Float => {
			let mut value = match script_field_current_value(field, fields, kind) {
				Some(ScriptFieldValue::Float(value)) => value,
				_ => 0.0,
			};
			ui.horizontal(|ui| {
				ui.monospace(&field.name);
				ui.label(":");
				ui.monospace(&field.ty);
				let response = ui.add(drag_value_f32(&mut value, 0.05));
				let edit = response_field_edit(&response);
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::Float(value));
				}
			});
		}
		ScriptFieldKind::String => {
			let mut value = match script_field_current_value(field, fields, kind) {
				Some(ScriptFieldValue::String(value)) => value,
				_ => String::new(),
			};
			ui.horizontal(|ui| {
				ui.monospace(&field.name);
				ui.label(":");
				ui.monospace(&field.ty);
				let response = ui.text_edit_singleline(&mut value);
				let edit = response_field_edit(&response);
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::String(value));
				}
			});
		}
		ScriptFieldKind::Vec2 => {
			let mut value = match current_value {
				Some(ScriptFieldValue::Vec2(value)) => value,
				_ => Vec2::ZERO,
			};
			ui.horizontal(|ui| {
				ui.label(&field.name);
				let x_response = ui.add(drag_value_f32(&mut value.x, 0.05));
				let y_response = ui.add(drag_value_f32(&mut value.y, 0.05));
				let mut edit = response_field_edit(&x_response);
				edit.include(response_field_edit(&y_response));
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::Vec2(value));
				}
			});
		}
		ScriptFieldKind::Vec3 => {
			let mut value = match current_value {
				Some(ScriptFieldValue::Vec3(value)) => value,
				_ => Vec3::ZERO,
			};
			ui.horizontal(|ui| {
				ui.label(&field.name);
				let x_response = ui.add(drag_value_f32(&mut value.x, 0.05));
				let y_response = ui.add(drag_value_f32(&mut value.y, 0.05));
				let z_response = ui.add(drag_value_f32(&mut value.z, 0.05));
				let mut edit = response_field_edit(&x_response);
				edit.include(response_field_edit(&y_response));
				edit.include(response_field_edit(&z_response));
				field_edit.include(edit);
				if edit.changed {
					fields.insert(field.name.clone(), ScriptFieldValue::Vec3(value));
				}
			});
		}
	}

	field_edit
}

fn script_field_current_value(
	field: &ScriptFieldMetadata,
	fields: &std::collections::BTreeMap<String, ScriptFieldValue>,
	kind: ScriptFieldKind,
) -> Option<ScriptFieldValue> {
	fields
		.get(&field.name)
		.and_then(|value| script_field_value_for_kind(value, kind))
		.or_else(|| {
			field
				.default_value
				.as_ref()
				.and_then(|value| script_field_value_for_kind(value, kind))
		})
}

fn script_field_display_value<'a>(
	field: &'a ScriptFieldMetadata,
	fields: &'a std::collections::BTreeMap<String, ScriptFieldValue>,
) -> Option<&'a ScriptFieldValue> {
	fields.get(&field.name).or(field.default_value.as_ref())
}

fn script_field_value_for_kind(value: &ScriptFieldValue, kind: ScriptFieldKind) -> Option<ScriptFieldValue> {
	match (kind, value) {
		(ScriptFieldKind::Bool, ScriptFieldValue::Bool(value)) => Some(ScriptFieldValue::Bool(*value)),
		(ScriptFieldKind::Int, ScriptFieldValue::Int(value)) => Some(ScriptFieldValue::Int(*value)),
		(ScriptFieldKind::UInt, ScriptFieldValue::UInt(value)) => Some(ScriptFieldValue::UInt(*value)),
		(ScriptFieldKind::Float, ScriptFieldValue::Float(value)) => Some(ScriptFieldValue::Float(*value)),
		(ScriptFieldKind::String, ScriptFieldValue::String(value)) => Some(ScriptFieldValue::String(value.clone())),
		(ScriptFieldKind::Vec2, ScriptFieldValue::Vec2(value)) => Some(ScriptFieldValue::Vec2(*value)),
		(ScriptFieldKind::Vec3, ScriptFieldValue::Vec3(value)) => Some(ScriptFieldValue::Vec3(*value)),
		_ => None,
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptFieldKind {
	Bool,
	Int,
	UInt,
	Float,
	String,
	Vec2,
	Vec3,
}

fn script_field_kind(type_name: &str) -> Option<ScriptFieldKind> {
	match type_name {
		"bool" | "Boolean" => Some(ScriptFieldKind::Bool),
		"int" | "Int32" => Some(ScriptFieldKind::Int),
		"uint" | "UInt32" => Some(ScriptFieldKind::UInt),
		"float" | "Single" => Some(ScriptFieldKind::Float),
		"string" | "String" => Some(ScriptFieldKind::String),
		"Vec2" | "Vector2" => Some(ScriptFieldKind::Vec2),
		"Vec3" | "Vector3" => Some(ScriptFieldKind::Vec3),
		_ => None,
	}
}

fn show_script_field_readonly(ui: &mut Ui, field: &ScriptFieldMetadata, value: Option<&ScriptFieldValue>) {
	ui.horizontal(|ui| {
		ui.monospace(&field.name);
		ui.label(":");
		ui.monospace(&field.ty);
	});
	ui.colored_label(egui::Color32::YELLOW, "Unsupported field type. Value is read-only.");
	if let Some(value) = value {
		ui.monospace(format!("{value:?}"));
	}
}

fn drag_value_f32(value: &mut f32, speed: f64) -> egui::DragValue<'_> {
	egui::DragValue::new(value)
		.speed(speed)
		.custom_parser(parse_f64_zero_empty)
}

fn drag_value_i32(value: &mut i32, speed: f64) -> egui::DragValue<'_> {
	egui::DragValue::new(value)
		.speed(speed)
		.custom_parser(parse_i32_zero_empty)
}

fn drag_value_u32(value: &mut u32, speed: f64) -> egui::DragValue<'_> {
	egui::DragValue::new(value)
		.speed(speed)
		.custom_parser(parse_u32_zero_empty)
}

fn parse_f64_zero_empty(text: &str) -> Option<f64> {
	let text = text.trim();
	if text.is_empty() {
		Some(0.0)
	} else {
		text.parse::<f64>().ok().filter(|value| value.is_finite())
	}
}

fn parse_i32_zero_empty(text: &str) -> Option<f64> {
	let text = text.trim();
	if text.is_empty() {
		Some(0.0)
	} else {
		text.parse::<i32>().ok().map(f64::from)
	}
}

fn parse_u32_zero_empty(text: &str) -> Option<f64> {
	let text = text.trim();
	if text.is_empty() {
		Some(0.0)
	} else {
		text.parse::<u32>().ok().map(f64::from)
	}
}

fn response_field_edit(response: &egui::Response) -> FieldEdit {
	FieldEdit {
		changed: response.changed(),
		committed: (response.changed() && response.clicked())
			|| response.lost_focus()
			|| response.drag_stopped()
			|| (response.has_focus() && response.ctx.input(|input| input.key_pressed(egui::Key::Enter))),
	}
}

fn vec3_editor(ui: &mut Ui, label: &str, value: &mut Vec3) -> FieldEdit {
	let mut field_edit = FieldEdit::default();
	ui.horizontal(|ui| {
		ui.label(label);
		let x = ui.add(drag_value_f32(&mut value.x, 0.05));
		let y = ui.add(drag_value_f32(&mut value.y, 0.05));
		let z = ui.add(drag_value_f32(&mut value.z, 0.05));
		field_edit.include(response_field_edit(&x));
		field_edit.include(response_field_edit(&y));
		field_edit.include(response_field_edit(&z));
	});
	field_edit
}

fn vec2_editor(ui: &mut Ui, label: &str, value: &mut Vec2) -> FieldEdit {
	let mut field_edit = FieldEdit::default();
	ui.horizontal(|ui| {
		ui.label(label);
		ui.label("X");
		let x = ui.add(drag_value_f32(&mut value.x, 0.05));
		ui.label("Y");
		let y = ui.add(drag_value_f32(&mut value.y, 0.05));
		field_edit.include(response_field_edit(&x));
		field_edit.include(response_field_edit(&y));
	});
	field_edit
}

fn drag_i32(ui: &mut Ui, label: &str, value: &mut i32) -> FieldEdit {
	let mut field_edit = FieldEdit::default();
	ui.horizontal(|ui| {
		ui.label(label);
		let response = ui.add(drag_value_i32(value, 1.0));
		field_edit = response_field_edit(&response);
	});
	field_edit
}

fn drag_f32(ui: &mut Ui, label: &str, value: &mut f32) -> FieldEdit {
	let mut field_edit = FieldEdit::default();
	ui.horizontal(|ui| {
		ui.label(label);
		let response = ui.add(drag_value_f32(value, 0.05));
		field_edit = response_field_edit(&response);
	});
	field_edit
}

fn rigidbody_type_option(ui: &mut Ui, value: &mut RigidBodyType, option: RigidBodyType) -> FieldEdit {
	let selected = rigidbody_type_matches(*value, option);
	if ui.selectable_label(selected, rigidbody_type_label(option)).clicked() {
		*value = option;
		return FieldEdit::changed(!selected);
	}

	FieldEdit::default()
}

const fn rigidbody_type_matches(left: RigidBodyType, right: RigidBodyType) -> bool {
	matches!(
		(left, right),
		(RigidBodyType::Static, RigidBodyType::Static)
			| (RigidBodyType::Dynamic, RigidBodyType::Dynamic)
			| (
				RigidBodyType::KinematicPositionBased,
				RigidBodyType::KinematicPositionBased
			) | (
			RigidBodyType::KinematicVelocityBased,
			RigidBodyType::KinematicVelocityBased
		)
	)
}

const fn rigidbody_type_label(value: RigidBodyType) -> &'static str {
	match value {
		RigidBodyType::Static => "Static",
		RigidBodyType::Dynamic => "Dynamic",
		RigidBodyType::KinematicPositionBased => "Kinematic Position Based",
		RigidBodyType::KinematicVelocityBased => "Kinematic Velocity Based",
	}
}

const fn collision_detection_label(value: CollisionDetection) -> &'static str {
	match value {
		CollisionDetection::Discrete => "Discrete",
		CollisionDetection::Continuous => "Continuous",
	}
}

const fn sprite_color_mode_label(mode: SpriteColorMode) -> &'static str {
	match mode {
		SpriteColorMode::None => "None",
		SpriteColorMode::Multiply => "Multiply",
		SpriteColorMode::GrayscaleTint => "Grayscale Tint",
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColliderShapeKind {
	Box,
	Circle,
	Capsule,
	ConvexPolygon,
}

impl ColliderShapeKind {
	const fn from_shape(shape: &ColliderShape) -> Self {
		match shape {
			ColliderShape::Box { .. } => Self::Box,
			ColliderShape::Circle { .. } => Self::Circle,
			ColliderShape::Capsule { .. } => Self::Capsule,
			ColliderShape::ConvexPolygon { .. } => Self::ConvexPolygon,
		}
	}

	const fn label(self) -> &'static str {
		match self {
			Self::Box => "Box",
			Self::Circle => "Circle",
			Self::Capsule => "Capsule",
			Self::ConvexPolygon => "Convex Polygon",
		}
	}

	fn default_shape(self) -> ColliderShape {
		match self {
			Self::Box => ColliderShape::Box {
				half_extents: Vec2::new(0.5, 0.5),
			},
			Self::Circle => ColliderShape::Circle { radius: 0.5 },
			Self::Capsule => ColliderShape::Capsule {
				half_height: 0.5,
				radius: 0.25,
			},
			Self::ConvexPolygon => ColliderShape::ConvexPolygon {
				points: vec![Vec2::new(-0.5, -0.5), Vec2::new(0.5, -0.5), Vec2::new(0.0, 0.5)],
			},
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CameraProjectionKind {
	Orthographic,
	Perspective,
}

impl CameraProjectionKind {
	const fn label(self) -> &'static str {
		match self {
			Self::Orthographic => "Orthographic",
			Self::Perspective => "Perspective",
		}
	}
}

fn record_scene_edit(context: &mut EditorPanelContext<'_>, edit: impl FnOnce(&mut ze_ecs::Scene)) {
	let before = context.scene.snapshot();
	edit(context.scene);
	let after = context.scene.snapshot();
	context
		.undo_redo
		.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
}

impl InspectorPanel {
	fn show_ui_button(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut button: UIButton,
	) {
		let response = show_removable_component(ui, InspectableComponent::UIButton.label(), |ui| {
			let mut field_edit = FieldEdit::default();

			ui.horizontal(|ui| {
				ui.label("Text");
				let response = ui.text_edit_singleline(&mut button.text);
				field_edit.include(response_field_edit(&response));
			});

			field_edit.include(drag_f32(ui, "Font Size", &mut button.font_size));

			ui.horizontal(|ui| {
				ui.label("X");
				field_edit.include(response_field_edit(&ui.add(egui::DragValue::new(&mut button.rect.x))));
				ui.label("Y");
				field_edit.include(response_field_edit(&ui.add(egui::DragValue::new(&mut button.rect.y))));
			});
			ui.horizontal(|ui| {
				ui.label("Width");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut button.rect.width).range(0.0..=f32::MAX)),
				));
				ui.label("Height");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut button.rect.height).range(0.0..=f32::MAX)),
				));
			});

			field_edit.include(drag_i32(ui, "Z Index", &mut button.z_index));

			ui.horizontal(|ui| {
				ui.label("Color R");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut button.color[0]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("G");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut button.color[1]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("B");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut button.color[2]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("A");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut button.color[3]).speed(0.01).range(0.0..=1.0)),
				));
			});
			ui.horizontal(|ui| {
				ui.label("Hover R");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut button.hover_color[0])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
				ui.label("G");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut button.hover_color[1])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
				ui.label("B");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut button.hover_color[2])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
				ui.label("A");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut button.hover_color[3])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
			});

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut UIButton>(entity) {
					**current = button;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::UIButton);
	}

	fn show_ui_bar(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>, entity: EntityId, mut bar: UIBar) {
		let response = show_removable_component(ui, InspectableComponent::UIBar.label(), |ui| {
			let mut field_edit = FieldEdit::default();

			field_edit.include(drag_f32(ui, "Current", &mut bar.current));
			field_edit.include(drag_f32(ui, "Max", &mut bar.max));

			ui.horizontal(|ui| {
				ui.label("X");
				field_edit.include(response_field_edit(&ui.add(egui::DragValue::new(&mut bar.rect.x))));
				ui.label("Y");
				field_edit.include(response_field_edit(&ui.add(egui::DragValue::new(&mut bar.rect.y))));
			});
			ui.horizontal(|ui| {
				ui.label("Width");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.rect.width).range(0.0..=f32::MAX)),
				));
				ui.label("Height");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.rect.height).range(0.0..=f32::MAX)),
				));
			});

			field_edit.include(drag_i32(ui, "Z Index", &mut bar.z_index));

			let mut label_text = bar.text.clone().unwrap_or_default();
			let mut has_label = bar.text.is_some();
			field_edit.include(response_field_edit(&ui.checkbox(&mut has_label, "Show label")));
			ui.horizontal(|ui| {
				ui.label("Label");
				if has_label {
					let response = ui.text_edit_singleline(&mut label_text);
					field_edit.include(response_field_edit(&response));
				} else {
					ui.label("None");
				}
			});
			bar.text = has_label.then_some(label_text);

			ui.horizontal(|ui| {
				ui.label("Color R");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.color[0]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("G");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.color[1]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("B");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.color[2]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("A");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.color[3]).speed(0.01).range(0.0..=1.0)),
				));
			});
			ui.horizontal(|ui| {
				ui.label("BG R");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.bg_color[0]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("G");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.bg_color[1]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("B");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.bg_color[2]).speed(0.01).range(0.0..=1.0)),
				));
				ui.label("A");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut bar.bg_color[3]).speed(0.01).range(0.0..=1.0)),
				));
			});

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut UIBar>(entity) {
					**current = bar;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::UIBar);
	}

	fn show_ui_text(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		mut text_component: UIText,
	) {
		let response = show_removable_component(ui, InspectableComponent::UIText.label(), |ui| {
			let mut field_edit = FieldEdit::default();

			ui.horizontal(|ui| {
				ui.label("Text");
				let response = ui.text_edit_singleline(&mut text_component.text);
				field_edit.include(response_field_edit(&response));
			});

			// A non-positive font_size hangs cosmic-text's line-layout (confirmed via
			// debugger), so this field can't reuse the unbounded drag_f32 helper.
			ui.horizontal(|ui| {
				ui.label("Font Size");
				let response = ui.add(drag_value_f32(&mut text_component.font_size, 0.05).range(1.0..=f32::MAX));
				field_edit.include(response_field_edit(&response));
			});

			ui.horizontal(|ui| {
				ui.label("X");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut text_component.rect.x)),
				));
				ui.label("Y");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut text_component.rect.y)),
				));
			});
			ui.horizontal(|ui| {
				ui.label("Width");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut text_component.rect.width).range(0.0..=f32::MAX)),
				));
				ui.label("Height");
				field_edit.include(response_field_edit(
					&ui.add(egui::DragValue::new(&mut text_component.rect.height).range(0.0..=f32::MAX)),
				));
			});

			field_edit.include(drag_i32(ui, "Z Index", &mut text_component.z_index));

			ui.horizontal(|ui| {
				ui.label("Color R");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut text_component.color[0])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
				ui.label("G");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut text_component.color[1])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
				ui.label("B");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut text_component.color[2])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
				ui.label("A");
				field_edit.include(response_field_edit(
					&ui.add(
						egui::DragValue::new(&mut text_component.color[3])
							.speed(0.01)
							.range(0.0..=1.0),
					),
				));
			});

			self.apply_field_edit(context, field_edit, |scene| {
				if let Ok(mut current) = scene.world_mut().get::<&mut UIText>(entity) {
					**current = text_component;
				}
			});
		});
		self.show_component_context_menu(&response, entity, context, InspectableComponent::UIText);
	}
}

impl Panel for InspectorPanel {
	fn name(&self) -> &'static str { "Inspector" }

	fn show(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		if *context.script_classes_dirty {
			self.script_classes_cache = None;
			*context.script_classes_dirty = false;
		}

		let Some(selection) = context.selection.clone() else {
			ui.centered_and_justified(|ui| {
				ui.label("Nothing selected");
			});

			return;
		};

		match selection {
			EditorSelection::Entity(entity) => {
				let context_menu_rect = ui.max_rect();
				let context_menu_response = ui.interact(
					context_menu_rect,
					ui.id().with(("inspector_entity_context", entity)),
					egui::Sense::click(),
				);
				self.show_entity(ui, entity, context);
				self.show_inspector_context_menu(&context_menu_response, entity, context);
			}
			EditorSelection::Asset(path) => {
				self.show_asset(ui, &path, context);
			}
		}
	}

	fn save_project_if_dirty(&mut self) -> Option<Result<Project, String>> {
		self.flush_dirty_text_buffer();

		let state = self.project_settings.as_mut()?;
		if !state.dirty {
			return None;
		}

		Some(save_project_settings_state(state))
	}

	fn has_unsaved_project_changes(&self) -> bool {
		self.project_settings.as_ref().is_some_and(|state| state.dirty)
			|| (self.opened_code_path.is_some() && self.code_buffer != self.code_buffer_snapshot)
	}
}
