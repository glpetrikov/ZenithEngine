use std::{fs, path::Path};

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AssetRef, CellId};

pub const ANIMATION_CLIP_EXTENSION: &str = "animationclip.json";
const ANIMATION_CLIP_VERSION: &str = "1";

/// An ordered sequence of `TextureSheet` cells (a Uniform Grid sheet only --
/// see `Animator`/`AnimationSystem` for the runtime playback logic), played
/// back at a fixed per-frame duration.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AnimationClip {
	pub version: String,
	pub texture_sheet: AssetRef,
	/// Ordered, repeats allowed, may be empty while a clip is still being
	/// authored.
	pub frames: Vec<CellId>,
	pub frame_duration_ms: f32,
	/// Named `loop_animation` rather than `loop` -- a reserved word.
	pub loop_animation: bool,
}

impl AnimationClip {
	pub fn new(texture_sheet: AssetRef) -> Self {
		Self {
			version: ANIMATION_CLIP_VERSION.to_string(),
			texture_sheet,
			frames: Vec::new(),
			frame_duration_ms: 100.0,
			loop_animation: true,
		}
	}

	pub fn load(path: &Path) -> Result<Self> {
		let text = fs::read_to_string(path)?;
		Ok(serde_json::from_str(&text)?)
	}

	pub fn save(&self, path: &Path) -> Result<()> {
		let text = serde_json::to_string_pretty(self)?;
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(path, text)?;
		Ok(())
	}
}

/// Advances one animation tick shared by both `AnimationSystem` (runtime) and
/// the Animation Clip panel's playback preview (editor), so the two never
/// drift out of sync. Uses a `while` loop (not a single `if`) so a large `dt`
/// (e.g. after a stall) still catches up correctly instead of only ever
/// advancing one frame per call.
pub fn step_animation_frame(
	frame_index: &mut usize,
	elapsed_secs: &mut f32,
	dt: f32,
	frame_count: usize,
	frame_duration_ms: f32,
	loop_animation: bool,
) {
	if frame_count == 0 {
		return;
	}

	*elapsed_secs += dt;
	let frame_duration_secs = (frame_duration_ms / 1000.0).max(0.001);

	while *elapsed_secs >= frame_duration_secs {
		*elapsed_secs -= frame_duration_secs;
		let next = *frame_index + 1;
		if next < frame_count {
			*frame_index = next;
		} else if loop_animation {
			*frame_index = 0;
		} else {
			*frame_index = frame_count - 1;
			*elapsed_secs = 0.0;
			break;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn loops_back_to_first_frame() {
		let mut frame_index = 2;
		let mut elapsed = 0.0;
		step_animation_frame(&mut frame_index, &mut elapsed, 0.1, 3, 100.0, true);
		assert_eq!(frame_index, 0);
	}

	#[test]
	fn clamps_on_last_frame_when_not_looping() {
		let mut frame_index = 2;
		let mut elapsed = 0.0;
		step_animation_frame(&mut frame_index, &mut elapsed, 0.1, 3, 100.0, false);
		assert_eq!(frame_index, 2);
		assert_eq!(elapsed, 0.0);
	}

	#[test]
	fn catches_up_across_multiple_frames_in_one_large_dt() {
		let mut frame_index = 0;
		let mut elapsed = 0.0;
		step_animation_frame(&mut frame_index, &mut elapsed, 0.35, 4, 100.0, true);
		assert_eq!(frame_index, 3);
	}

	#[test]
	fn round_trips_through_json() {
		let clip = AnimationClip::new(AssetRef::game("texture_sheets/player.texturesheet.json"));
		let json = serde_json::to_string(&clip).expect("serialize");
		let round_tripped: AnimationClip = serde_json::from_str(&json).expect("deserialize");
		assert_eq!(round_tripped.frame_duration_ms, 100.0);
		assert!(round_tripped.loop_animation);
		assert!(round_tripped.frames.is_empty());
	}
}
