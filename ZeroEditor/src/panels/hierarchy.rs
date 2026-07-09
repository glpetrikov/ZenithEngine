use egui::{Key, Sense, Ui};
use ze_ecs::{
	Children, Collider, Component, EditorOnly, EntitiesView, EntityId, Inactive, Name, Parent, PhysicsSettings,
	RigidBody, Tag, Transform, ze_entity_id::ZeEntityId,
};
use ze_renderer::{Camera, Sprite};
use ze_scripting_cs::Scripts;

use super::{EditorPanelContext, EditorSelection, Panel};
use crate::undo_redo::SceneSnapshotCommand;

#[derive(Debug, Default)]
pub struct SceneHierarchyPanel {
	renaming_entity: Option<EntityId>,
	rename_buffer: String,
	rename_original_name: String,
	rename_focus_requested: bool,
	is_new_unconfirmed: bool,
	collapsed_entities: Vec<ZeEntityId>,
}

impl SceneHierarchyPanel {
	pub fn new() -> Self { Self::default() }
}

impl Panel for SceneHierarchyPanel {
	fn name(&self) -> &'static str { "Hierarchy" }

	fn show(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		let previous_interact_size = ui.spacing_mut().interact_size;
		ui.spacing_mut().interact_size.y = 16.0;

		for entity in root_entities(context.scene) {
			self.show_entity(ui, context, entity, 0, &mut Vec::new());
		}

		ui.spacing_mut().interact_size = previous_interact_size;

		let empty_area = ui.available_rect_before_wrap();
		if empty_area.is_positive() {
			let response = ui.allocate_rect(empty_area, Sense::click());
			if response.clicked() {
				*context.selection = None;
			}
			response.context_menu(|ui| {
				if ui.button("Create Entity").clicked() {
					create_and_start_rename(self, context, None, "Entity");
					ui.close();
				}
			});
			if let Some(payload) = response.dnd_release_payload::<ZeEntityId>() {
				set_parent(context, EntityId::from(*payload), None);
			}
		}
	}
}

impl SceneHierarchyPanel {
	fn show_entity(
		&mut self,
		ui: &mut Ui,
		context: &mut EditorPanelContext<'_>,
		entity: EntityId,
		depth: usize,
		visited: &mut Vec<EntityId>,
	) {
		if !hierarchy_visible(context.scene, entity) {
			return;
		}
		if visited.contains(&entity) {
			ze_log::warn!("Hierarchy tree detected a cycle at entity {}", entity.index());
			return;
		}
		visited.push(entity);

		let children = children_of(context.scene, entity);
		let has_children = !children.is_empty();
		let selected = context.selection.as_ref().is_some_and(
			|selected| matches!(selected, EditorSelection::Entity(selected_entity) if *selected_entity == entity),
		);
		let collapsed = self.is_collapsed(entity);

		let is_renaming = self.renaming_entity == Some(entity);
		let response = ui
			.horizontal(|ui| {
				ui.add_space(depth as f32 * 14.0);
				if has_children {
					let marker = if collapsed { ">" } else { "v" };
					if ui.add_sized([16.0, 18.0], egui::Button::new(marker).small()).clicked() {
						self.toggle_collapsed(entity);
					}
				} else {
					ui.add_space(18.0);
				}

				if is_renaming {
					let text_response = ui.add(
						egui::TextEdit::singleline(&mut self.rename_buffer)
							.desired_width(160.0)
							.id_source(("hierarchy_rename", entity.index(), entity.r#gen())),
					);
					if self.rename_focus_requested {
						text_response.request_focus();
						self.rename_focus_requested = false;
					}
					if text_response.clicked() {
						text_response.request_focus();
					}

					let enter_pressed = ui.input(|input| input.key_pressed(Key::Enter));
					let confirm_pressed = enter_pressed && (text_response.has_focus() || text_response.lost_focus());
					let cancel_pressed = text_response.has_focus() && ui.input(|input| input.key_pressed(Key::Escape));
					if confirm_pressed {
						text_response.surrender_focus();
						rename_entity(context, entity, &self.rename_buffer);
						self.finish_rename();
					} else if cancel_pressed || text_response.lost_focus() {
						text_response.surrender_focus();
						self.cancel_rename(context);
					}
					text_response
				} else {
					ui.add(
						egui::Button::new(entity_label(context.scene, entity))
							.selected(selected)
							.frame(false)
							.sense(Sense::click_and_drag()),
					)
				}
			})
			.inner;

		if !is_renaming && response.clicked() {
			*context.selection = Some(EditorSelection::Entity(entity));
		}

		if !is_renaming {
			response.dnd_set_drag_payload(ZeEntityId::from(entity));
			if let Some(payload) = response.dnd_release_payload::<ZeEntityId>() {
				set_parent(context, EntityId::from(*payload), Some(entity));
			}
		}

		if is_renaming && response.clicked() {
			*context.selection = Some(EditorSelection::Entity(entity));
		}

		response.context_menu(|ui| {
			if ui.button("Create").clicked() {
				create_and_start_rename(self, context, Some(entity), "Entity");
				ui.close();
			}
			if ui.button("Rename").clicked() {
				self.start_rename(context, entity);
				ui.close();
			}
			if ui.button("Duplicate").clicked() {
				duplicate_entity(context, entity);
				ui.close();
			}
			if ui.button("Delete Entity").clicked() {
				delete_entity(context, entity);
				ui.close();
			}
		});

		if !collapsed {
			for child in children {
				self.show_entity(ui, context, child, depth + 1, visited);
			}
		}
		visited.retain(|visited_entity| *visited_entity != entity);
	}

	fn is_collapsed(&self, entity: EntityId) -> bool {
		let entity = ZeEntityId::from(entity);
		self.collapsed_entities.contains(&entity)
	}

	fn toggle_collapsed(&mut self, entity: EntityId) {
		let entity = ZeEntityId::from(entity);
		if let Some(index) = self
			.collapsed_entities
			.iter()
			.position(|collapsed| *collapsed == entity)
		{
			self.collapsed_entities.remove(index);
		} else {
			self.collapsed_entities.push(entity);
		}
	}

	fn start_rename(&mut self, context: &mut EditorPanelContext<'_>, entity: EntityId) {
		self.is_new_unconfirmed = false;
		self.renaming_entity = Some(entity);
		self.rename_buffer = entity_label(context.scene, entity);
		self.rename_original_name = self.rename_buffer.clone();
		self.rename_focus_requested = true;
		*context.selection = Some(EditorSelection::Entity(entity));
	}

	fn finish_rename(&mut self) {
		self.renaming_entity = None;
		self.rename_original_name.clear();
		self.is_new_unconfirmed = false;
	}

	fn cancel_rename(&mut self, context: &mut EditorPanelContext<'_>) {
		if self.is_new_unconfirmed {
			if let Some(entity) = self.renaming_entity {
				let before = context.scene.snapshot();
				context.scene.destroy_entity(entity);
				let after = context.scene.snapshot();
				context
					.undo_redo
					.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
				*context.selection = None;
			}
			self.is_new_unconfirmed = false;
		}
		self.rename_buffer = self.rename_original_name.clone();
		self.finish_rename();
	}
}

fn create_and_start_rename(
	panel: &mut SceneHierarchyPanel,
	context: &mut EditorPanelContext<'_>,
	parent: Option<EntityId>,
	name: &str,
) -> EntityId {
	let before = context.scene.snapshot();
	let entity = context.scene.create_entity(name);
	context.scene.entity_mut(entity).add_component(Transform::default());
	if let Some(parent) = parent {
		attach_to_parent(context.scene, entity, parent);
	}
	let after = context.scene.snapshot();
	context
		.undo_redo
		.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
	panel.start_rename(context, entity);
	panel.is_new_unconfirmed = true;
	*context.selection = Some(EditorSelection::Entity(entity));
	entity
}

fn set_parent(context: &mut EditorPanelContext<'_>, child: EntityId, parent: Option<EntityId>) {
	if !entity_exists(context.scene, child) {
		ze_log::warn!("Hierarchy cannot reparent missing entity {}", child.index());
		return;
	}

	if let Some(parent) = parent {
		if !entity_exists(context.scene, parent) {
			ze_log::warn!("Hierarchy cannot parent to missing entity {}", parent.index());
			return;
		}
		if child == parent {
			ze_log::warn!("Hierarchy cannot parent an entity to itself");
			return;
		}
		if is_descendant(context.scene, parent, child) {
			ze_log::warn!("Hierarchy cannot parent an entity to one of its descendants");
			return;
		}
	}

	if parent_of(context.scene, child) == parent {
		return;
	}

	let world = context.scene.world_transform(child);

	let before = context.scene.snapshot();
	detach_from_parent(context.scene, child);

	if let Some(parent) = parent {
		attach_to_parent(context.scene, child, parent);
	}

	// Preserve world-space transform after reparenting: the world transform was
	// captured *before* the parent changed so that the composition is correct.
	// Then recompute local transform relative to the new parent so the child does
	// not visually move, rescale, or reorient.
	if let Some(world) = world {
		let _ = context.scene.set_world_transform(child, world);
	}

	let after = context.scene.snapshot();
	context
		.undo_redo
		.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
	*context.selection = Some(EditorSelection::Entity(child));
}

fn delete_entity(context: &mut EditorPanelContext<'_>, entity: EntityId) {
	let before = context.scene.snapshot();
	context.scene.destroy_entity(entity);
	let after = context.scene.snapshot();
	context
		.undo_redo
		.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
	*context.selection = None;
}

fn rename_entity(context: &mut EditorPanelContext<'_>, entity: EntityId, name: &str) {
	let name = name.trim();
	if name.is_empty() {
		ze_log::warn!("Hierarchy cannot rename entity to an empty name");
		return;
	}

	let before = context.scene.snapshot();
	if let Ok(mut entity_name) = context.scene.world_mut().get::<&mut Name>(entity) {
		entity_name.name = name.to_string();
	} else {
		context
			.scene
			.entity_mut(entity)
			.add_component(Name { name: name.to_string() });
	}
	let after = context.scene.snapshot();
	context
		.undo_redo
		.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
	*context.selection = Some(EditorSelection::Entity(entity));
}

fn duplicate_entity(context: &mut EditorPanelContext<'_>, source: EntityId) {
	let before = context.scene.snapshot();
	let source_name = entity_label(context.scene, source);
	let duplicate = context.scene.create_entity(&format!("{source_name} Copy"));

	copy_component::<Transform>(context.scene, source, duplicate, "Transform");
	copy_component::<Tag>(context.scene, source, duplicate, "Tag");
	copy_component::<Inactive>(context.scene, source, duplicate, "Inactive");
	copy_component::<Sprite>(context.scene, source, duplicate, "Sprite");
	copy_component::<Camera>(context.scene, source, duplicate, "Camera");
	copy_component::<RigidBody>(context.scene, source, duplicate, "RigidBody");
	copy_component::<Collider>(context.scene, source, duplicate, "Collider");
	copy_component::<Scripts>(context.scene, source, duplicate, "Scripts");

	if context.scene.world().get::<&Children>(source).is_ok() || context.scene.world().get::<&Parent>(source).is_ok() {
		ze_log::warn!("Hierarchy duplicate skipped Parent/Children relationship components");
	}

	let after = context.scene.snapshot();
	context
		.undo_redo
		.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
	*context.selection = Some(EditorSelection::Entity(duplicate));
}

fn copy_component<T>(scene: &mut ze_ecs::Scene, source: EntityId, target: EntityId, label: &str)
where
	T: Component + Clone + 'static,
{
	let Some(component) = cloned_component::<T>(scene, source) else {
		return;
	};

	scene.world_mut().add_component(target, (component,));
	ze_log::debug!("Hierarchy duplicate copied {label}");
}

fn cloned_component<T>(scene: &ze_ecs::Scene, entity: EntityId) -> Option<T>
where
	T: Component + Clone + 'static,
{
	scene.world().get::<&T>(entity).ok().map(|component| component.clone())
}

fn add_child_reference(scene: &mut ze_ecs::Scene, parent: EntityId, child: EntityId) {
	if let Ok(mut children) = scene.world_mut().get::<&mut Children>(parent) {
		if !children.ids.iter().any(|id| EntityId::from(*id) == child) {
			children.ids.push(ZeEntityId::from(child));
		}
	} else {
		scene.world_mut().add_component(
			parent,
			(Children {
				ids: vec![ZeEntityId::from(child)],
			},),
		);
	}
}

fn remove_child_reference(scene: &mut ze_ecs::Scene, parent: EntityId, child: EntityId) {
	let should_remove_children = if let Ok(mut children) = scene.world_mut().get::<&mut Children>(parent) {
		children.ids.retain(|id| EntityId::from(*id) != child);
		children.ids.is_empty()
	} else {
		false
	};

	if should_remove_children && let Err(error) = scene.entity_mut(parent).remove_component::<Children>() {
		ze_log::warn!(
			"Hierarchy failed to remove empty Children from entity {}: {error:?}",
			parent.index()
		);
	}
}

fn attach_to_parent(scene: &mut ze_ecs::Scene, child: EntityId, parent: EntityId) {
	scene.world_mut().add_component(
		child,
		(Parent {
			id: ZeEntityId::from(parent),
		},),
	);
	add_child_reference(scene, parent, child);
}

fn detach_from_parent(scene: &mut ze_ecs::Scene, child: EntityId) {
	let Some(parent) = cloned_component::<Parent>(scene, child).map(|parent| EntityId::from(parent.id)) else {
		return;
	};

	if let Err(error) = scene.entity_mut(child).remove_component::<Parent>() {
		ze_log::warn!(
			"Hierarchy failed to clear Parent from entity {}: {error:?}",
			child.index()
		);
	}
	remove_child_reference(scene, parent, child);
}

fn scene_entities(scene: &ze_ecs::Scene) -> Vec<EntityId> {
	scene.world().run(|entities: EntitiesView| entities.iter().collect())
}

fn root_entities(scene: &ze_ecs::Scene) -> Vec<EntityId> {
	let mut entities = scene_entities(scene)
		.into_iter()
		.filter(|entity| hierarchy_visible(scene, *entity))
		.filter(|entity| parent_of(scene, *entity).is_none_or(|parent| !hierarchy_visible(scene, parent)))
		.collect::<Vec<_>>();
	sort_entities(scene, &mut entities);
	entities
}

fn children_of(scene: &ze_ecs::Scene, parent: EntityId) -> Vec<EntityId> {
	let mut children = Vec::new();

	if let Ok(component) = scene.world().get::<&Children>(parent) {
		for id in &component.ids {
			let child = EntityId::from(*id);
			if entity_exists(scene, child) && hierarchy_visible(scene, child) && !children.contains(&child) {
				children.push(child);
			}
		}
	}

	for entity in scene_entities(scene) {
		if hierarchy_visible(scene, entity) && parent_of(scene, entity) == Some(parent) && !children.contains(&entity) {
			children.push(entity);
		}
	}

	sort_entities(scene, &mut children);
	children
}

fn parent_of(scene: &ze_ecs::Scene, entity: EntityId) -> Option<EntityId> {
	let parent = cloned_component::<Parent>(scene, entity).map(|parent| EntityId::from(parent.id))?;
	entity_exists(scene, parent).then_some(parent)
}

fn entity_exists(scene: &ze_ecs::Scene, entity: EntityId) -> bool { scene_entities(scene).contains(&entity) }

fn hierarchy_visible(scene: &ze_ecs::Scene, entity: EntityId) -> bool {
	scene.world().get::<&PhysicsSettings>(entity).is_err() && scene.world().get::<&EditorOnly>(entity).is_err()
}

fn is_descendant(scene: &ze_ecs::Scene, entity: EntityId, ancestor: EntityId) -> bool {
	let mut stack = children_of(scene, ancestor);
	let mut visited = Vec::new();

	while let Some(candidate) = stack.pop() {
		if candidate == entity {
			return true;
		}
		if visited.contains(&candidate) {
			continue;
		}
		visited.push(candidate);
		stack.extend(children_of(scene, candidate));
	}

	false
}

fn sort_entities(scene: &ze_ecs::Scene, entities: &mut [EntityId]) {
	entities.sort_by(|left, right| {
		entity_label(scene, *left)
			.to_lowercase()
			.cmp(&entity_label(scene, *right).to_lowercase())
			.then_with(|| left.index().cmp(&right.index()))
	});
}

fn entity_label(scene: &ze_ecs::Scene, entity: EntityId) -> String {
	scene
		.world()
		.get::<&Name>(entity)
		.map_or_else(|_| format!("Entity {}", entity.index()), |name| name.name.clone())
}
