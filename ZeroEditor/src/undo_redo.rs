use std::collections::VecDeque;

use ze_ecs::{SaveFile, Scene};

pub trait Command {
	fn execute(&mut self, scene: &mut Scene) -> ze_core::Result<()>;
	fn undo(&mut self, scene: &mut Scene) -> ze_core::Result<()>;
}

pub struct UndoRedoBuffer {
	undo_stack: VecDeque<Box<dyn Command>>,
	redo_stack: VecDeque<Box<dyn Command>>,
	max_capacity: usize,
}

impl UndoRedoBuffer {
	pub const fn new(max_capacity: usize) -> Self {
		Self {
			undo_stack: VecDeque::new(),
			redo_stack: VecDeque::new(),
			max_capacity,
		}
	}

	#[expect(
		dead_code,
		reason = "Some editor edits are applied before recording, but the command buffer also supports direct execution."
	)]
	pub fn execute(&mut self, mut command: Box<dyn Command>, scene: &mut Scene) {
		if let Err(error) = command.execute(scene) {
			ze_log::error!("failed to execute editor command: {error:?}");
			return;
		}

		self.push_undo(command);
		self.redo_stack.clear();
	}

	pub fn record_executed(&mut self, command: Box<dyn Command>) {
		self.push_undo(command);
		self.redo_stack.clear();
	}

	pub fn undo(&mut self, scene: &mut Scene) {
		let Some(mut command) = self.undo_stack.pop_back() else {
			return;
		};

		if let Err(error) = command.undo(scene) {
			ze_log::error!("failed to undo editor command: {error:?}");
			self.undo_stack.push_back(command);
			return;
		}

		self.redo_stack.push_back(command);
	}

	pub fn redo(&mut self, scene: &mut Scene) {
		let Some(mut command) = self.redo_stack.pop_back() else {
			return;
		};

		if let Err(error) = command.execute(scene) {
			ze_log::error!("failed to redo editor command: {error:?}");
			self.redo_stack.push_back(command);
			return;
		}

		self.push_undo(command);
	}

	pub fn undo_depth(&self) -> usize { self.undo_stack.len() }

	pub fn redo_depth(&self) -> usize { self.redo_stack.len() }

	fn push_undo(&mut self, command: Box<dyn Command>) {
		if self.max_capacity == 0 {
			return;
		}

		if self.undo_stack.len() == self.max_capacity {
			self.undo_stack.pop_front();
		}
		self.undo_stack.push_back(command);
	}
}

pub struct SceneSnapshotCommand {
	before: SaveFile,
	after: SaveFile,
}

impl SceneSnapshotCommand {
	pub const fn new(before: SaveFile, after: SaveFile) -> Self { Self { before, after } }
}

impl Command for SceneSnapshotCommand {
	fn execute(&mut self, scene: &mut Scene) -> ze_core::Result<()> { scene.restore_snapshot(self.after.clone()) }

	fn undo(&mut self, scene: &mut Scene) -> ze_core::Result<()> { scene.restore_snapshot(self.before.clone()) }
}
