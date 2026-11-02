//! Recording the events of a play and playing them back.

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

use crate::{Input, InputEvent};

/// The format number written into a recording.
const VERSION: u32 = 1;

/// The events of one frame and how long the frame was.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordedFrame {
	/// Seconds the frame lasted.
	pub dt: f32,
	pub events: Vec<InputEvent>,
}

/// Every event of a play, frame by frame. Play it back with [`Replay`] to get
/// the same input again: for a test that needs a player, for a bug that
/// happens only after a minute of play, for a demo.
///
/// Settings, actions and the cursor mode are not events and are not recorded:
/// play back on an `Input` that is set up the same way.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recording {
	version: u32,
	pub frames: Vec<RecordedFrame>,
}

impl Recording {
	#[must_use]
	pub const fn new(frames: Vec<RecordedFrame>) -> Self {
		Self {
			version: VERSION,
			frames,
		}
	}

	/// Seconds the whole recording lasts.
	#[must_use]
	pub fn duration(&self) -> f32 {
		self.frames.iter().map(|frame| frame.dt).sum()
	}

	/// The recording as JSON text, for a file.
	///
	/// # Errors
	/// `Json` if a value cannot be written (a `NaN` in an event, say).
	pub fn to_json(&self) -> ZResult<String> {
		Ok(serde_json::to_string(self)?)
	}

	/// A recording from the text `to_json` wrote.
	///
	/// # Errors
	/// `Json` for text that is not a recording, `InvalidSettings` for a
	/// recording of another version.
	pub fn from_json(text: &str) -> ZResult<Self> {
		let recording: Self = serde_json::from_str(text)?;
		if recording.version != VERSION {
			return Err(ZenithError::InvalidSettings(format!(
				"an input recording of version {}, this engine reads version {VERSION}",
				recording.version
			)));
		}
		Ok(recording)
	}
}

/// Plays a [`Recording`] back, one frame at a time.
///
/// ```
/// use zenith_input::{Input, InputEvent, RecordedFrame, Recording, Replay, ZKeyCode};
///
/// let down = InputEvent::KeyDown(ZKeyCode::W);
/// let up = InputEvent::KeyUp(ZKeyCode::W);
/// let frame = |event| RecordedFrame {
/// 	dt: 0.016,
/// 	events: vec![event],
/// };
/// let mut replay = Replay::new(Recording::new(vec![frame(down), frame(up)]));
/// let mut input = Input::default();
/// // Each frame runs here (`world.update_systems(dt)`), which also ends it.
/// while let Some(dt) = replay.advance(&mut input) {
/// 	input.end_frame(dt);
/// }
/// assert!(!input.is_key_pressed(ZKeyCode::W));
/// ```
#[derive(Clone, Debug)]
pub struct Replay {
	recording: Recording,
	next: usize,
}

impl Replay {
	#[must_use]
	pub const fn new(recording: Recording) -> Self {
		Self { recording, next: 0 }
	}

	/// Gives the events of the next frame to `input` and returns how long the
	/// frame lasted; `None` once the recording is over.
	pub fn advance(&mut self, input: &mut Input) -> Option<f32> {
		let frame = self.recording.frames.get(self.next)?;
		self.next += 1;
		for event in &frame.events {
			input.apply(event.clone());
		}
		Some(frame.dt)
	}

	#[must_use]
	pub const fn is_finished(&self) -> bool {
		self.next >= self.recording.frames.len()
	}

	/// How many frames have been played.
	#[must_use]
	pub const fn frame(&self) -> usize {
		self.next
	}

	/// Starts again from the first frame.
	pub const fn restart(&mut self) {
		self.next = 0;
	}
}

#[cfg(test)]
mod tests {
	use glam::Vec2;

	use super::*;
	use crate::{ZGamePad, ZKeyCode};

	fn play() -> Recording {
		Recording::new(vec![
			RecordedFrame {
				dt: 0.5,
				events: vec![
					InputEvent::KeyDown(ZKeyCode::W),
					InputEvent::MouseMoved(Vec2::new(1.0, 2.0)),
				],
			},
			RecordedFrame {
				dt: 0.25,
				events: vec![InputEvent::GamepadButton {
					id: 0,
					button: ZGamePad::South,
					pressed: true,
				}],
			},
			RecordedFrame {
				dt: 0.25,
				events: vec![InputEvent::KeyUp(ZKeyCode::W), InputEvent::Text("hi".into())],
			},
		])
	}

	#[test]
	fn a_recording_survives_json() -> ZResult<()> {
		let recording = play();
		assert_eq!(Recording::from_json(&recording.to_json()?)?, recording, "round trip");
		assert!((recording.duration() - 1.0).abs() < 1e-6, "duration");
		assert!(Recording::from_json("{").is_err(), "not JSON");
		assert!(
			matches!(
				Recording::from_json(r#"{"version":99,"frames":[]}"#),
				Err(ZenithError::InvalidSettings(_))
			),
			"another version"
		);
		Ok(())
	}

	#[test]
	fn what_is_recorded_plays_back_the_same() {
		let mut recorded = Input::default();
		recorded.start_recording();
		recorded.apply(InputEvent::KeyDown(ZKeyCode::W));
		recorded.apply(InputEvent::MouseMoved(Vec2::new(3.0, 4.0)));
		recorded.end_frame(0.5);
		recorded.apply(InputEvent::KeyUp(ZKeyCode::W));
		recorded.end_frame(0.25);
		let recording = recorded.stop_recording();
		assert_eq!(recording.frames.len(), 2, "two frames");
		assert!(!recorded.is_recording(), "stopped");

		let mut replayed = Input::default();
		let mut replay = Replay::new(recording);
		assert_eq!(replay.advance(&mut replayed), Some(0.5), "first frame");
		assert!(replayed.is_key_just_pressed(ZKeyCode::W), "its events were applied");
		assert_eq!(replayed.mouse_position(), Some(Vec2::new(3.0, 4.0)), "all of them");
		replayed.end_frame(0.5);
		assert_eq!(replay.advance(&mut replayed), Some(0.25), "second frame");
		assert!(replayed.is_key_just_released(ZKeyCode::W), "released");
		assert!(replay.is_finished(), "that was all");
		assert_eq!(replay.advance(&mut replayed), None, "nothing more");
		replay.restart();
		assert_eq!(replay.frame(), 0, "restarted");
	}

	#[test]
	fn events_after_the_last_frame_are_kept_when_stopping() {
		let mut input = Input::default();
		input.start_recording();
		input.apply(InputEvent::KeyDown(ZKeyCode::A));
		let recording = input.stop_recording();
		assert_eq!(recording.frames.len(), 1, "the unfinished frame is a frame");
		assert!(
			recording.frames[0].events.contains(&InputEvent::KeyDown(ZKeyCode::A)),
			"with its event"
		);
	}
}
