//! `RenderSettings`: how the renderer opens the GPU and shows its frames.
//!
//! A resource, apart from the render graph (what is drawn). It is plain data
//! that can be saved: the enums here are the renderer's own, not wgpu's, so a
//! saved file does not change with the version of wgpu.
//!
//! Some settings take effect at once (how frames are shown); the others are
//! used when the GPU is opened, and wait for the next start (see each one).

use bevy_ecs::resource::Resource;
use serde::{Deserialize, Serialize};

/// How the renderer opens the GPU and shows its frames.
#[derive(Resource, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RenderSettings {
	/// The fast GPU or the one that spends less. At the next start.
	pub power: PowerPreference,
	/// The graphics API. Ignored in a browser, which only has WebGPU. At the
	/// next start.
	pub backend: Backend,
	/// Draw on the processor when there is no GPU to draw on. At the next
	/// start.
	pub fallback_adapter: bool,
	/// Whether the GPU keeps memory for speed or saves it. At the next start.
	pub memory: MemoryHint,
	/// The checks of the graphics API (slow, for finding mistakes). `None`:
	/// on in a debug build, off in a release one. At the next start.
	pub validation: Option<bool>,
	/// How frames wait for the screen. At once.
	pub present_mode: PresentMode,
	/// How the window is blended with what is behind it. At once.
	pub alpha_mode: AlphaMode,
	/// How many frames may wait to be shown: more is smoother, fewer answers
	/// the player sooner. At once.
	pub max_frame_latency: u32,
}

impl Default for RenderSettings {
	fn default() -> Self {
		Self {
			power: PowerPreference::HighPerformance,
			backend: Backend::Auto,
			fallback_adapter: false,
			memory: MemoryHint::Performance,
			validation: None,
			present_mode: PresentMode::AutoVsync,
			alpha_mode: AlphaMode::Opaque,
			max_frame_latency: 2,
		}
	}
}

impl RenderSettings {
	/// Do `self` and `other` open the GPU the same way? If not, a change from
	/// one to the other waits for the next start.
	pub fn same_device(&self, other: &Self) -> bool {
		self.power == other.power
			&& self.backend == other.backend
			&& self.fallback_adapter == other.fallback_adapter
			&& self.memory == other.memory
			&& self.validation == other.validation
	}

	pub(crate) const fn backends(&self) -> wgpu::Backends {
		if cfg!(target_arch = "wasm32") {
			return wgpu::Backends::BROWSER_WEBGPU;
		}
		match self.backend {
			Backend::Auto => wgpu::Backends::PRIMARY,
			Backend::Vulkan => wgpu::Backends::VULKAN,
			Backend::Dx12 => wgpu::Backends::DX12,
			Backend::Metal => wgpu::Backends::METAL,
			Backend::Gl => wgpu::Backends::GL,
		}
	}

	/// What the build asks for, then these settings, then the environment
	/// (`WGPU_VALIDATION`, `WGPU_DEBUG`), so a developer can still override
	/// them.
	pub(crate) fn instance_flags(&self) -> wgpu::InstanceFlags {
		let checks = wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG;
		let mut flags = wgpu::InstanceFlags::from_build_config();
		match self.validation {
			Some(true) => flags |= checks,
			Some(false) => flags -= checks,
			None => {}
		}
		flags.with_env()
	}

	pub(crate) const fn power_preference(&self) -> wgpu::PowerPreference {
		match self.power {
			PowerPreference::None => wgpu::PowerPreference::None,
			PowerPreference::LowPower => wgpu::PowerPreference::LowPower,
			PowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
		}
	}

	pub(crate) const fn memory_hints(&self) -> wgpu::MemoryHints {
		match self.memory {
			MemoryHint::Performance => wgpu::MemoryHints::Performance,
			MemoryHint::MemoryUsage => wgpu::MemoryHints::MemoryUsage,
		}
	}

	/// The present mode to configure, among those the surface has. A mode it
	/// lacks falls back to the automatic one that is closest.
	pub(crate) fn pick_present_mode(&self, caps: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
		use wgpu::PresentMode as P;

		let (wanted, fallback) = match self.present_mode {
			PresentMode::AutoVsync => return P::AutoVsync,
			PresentMode::AutoNoVsync => return P::AutoNoVsync,
			PresentMode::Fifo => (P::Fifo, P::AutoVsync),
			PresentMode::FifoRelaxed => (P::FifoRelaxed, P::AutoVsync),
			PresentMode::Mailbox => (P::Mailbox, P::AutoNoVsync),
			PresentMode::Immediate => (P::Immediate, P::AutoNoVsync),
		};
		if caps.present_modes.contains(&wanted) {
			wanted
		} else {
			zenith_log::warn!("The surface has no present mode {wanted:?}: {fallback:?} instead");
			fallback
		}
	}

	/// The alpha mode to configure, among those the surface has.
	pub(crate) fn pick_alpha_mode(&self, caps: &wgpu::SurfaceCapabilities) -> wgpu::CompositeAlphaMode {
		use wgpu::CompositeAlphaMode as A;

		let preference: &[A] = match self.alpha_mode {
			AlphaMode::Auto => return A::Auto,
			AlphaMode::Opaque => &[A::Opaque],
			AlphaMode::Transparent => &[A::PreMultiplied, A::PostMultiplied, A::Inherit],
			AlphaMode::PreMultiplied => &[A::PreMultiplied],
			AlphaMode::PostMultiplied => &[A::PostMultiplied],
			AlphaMode::Inherit => &[A::Inherit],
		};
		preference
			.iter()
			.copied()
			.find(|mode| caps.alpha_modes.contains(mode))
			.unwrap_or_else(|| {
				zenith_log::warn!(
					"The surface has no alpha mode for {:?}: automatic instead",
					self.alpha_mode
				);
				A::Auto
			})
	}
}

/// See [`RenderSettings::power`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerPreference {
	/// Whatever the system gives.
	None,
	LowPower,
	#[default]
	HighPerformance,
}

/// See [`RenderSettings::backend`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
	/// The best the system has: Vulkan, DirectX 12 or Metal.
	#[default]
	Auto,
	Vulkan,
	Dx12,
	Metal,
	/// OpenGL: for machines whose drivers have nothing newer.
	Gl,
}

/// See [`RenderSettings::memory`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryHint {
	#[default]
	Performance,
	MemoryUsage,
}

/// See [`RenderSettings::present_mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresentMode {
	/// Waits for the screen, whatever the system calls it. Always there.
	#[default]
	AutoVsync,
	/// Does not wait, if the system can; waits otherwise. Always there.
	AutoNoVsync,
	/// Waits for the screen.
	Fifo,
	/// Waits, unless the frame is late: then it is shown at once (and may
	/// tear).
	FifoRelaxed,
	/// Does not wait and does not tear: the newest frame is shown.
	Mailbox,
	/// Does not wait: frames may tear.
	Immediate,
}

/// See [`RenderSettings::alpha_mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaMode {
	/// What the system chooses.
	Auto,
	/// The window hides what is behind it.
	#[default]
	Opaque,
	/// What is not drawn shows what is behind the window: the first of
	/// `PreMultiplied`, `PostMultiplied` and `Inherit` the system has.
	Transparent,
	PreMultiplied,
	PostMultiplied,
	Inherit,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn caps(
		present_modes: &[wgpu::PresentMode],
		alpha_modes: &[wgpu::CompositeAlphaMode],
	) -> wgpu::SurfaceCapabilities {
		wgpu::SurfaceCapabilities {
			present_modes: present_modes.to_vec(),
			alpha_modes: alpha_modes.to_vec(),
			..Default::default()
		}
	}

	#[test]
	fn a_present_mode_the_surface_lacks_falls_back() {
		use wgpu::PresentMode as P;
		let fifo_only = caps(&[P::Fifo], &[]);
		let settings = |present_mode| RenderSettings {
			present_mode,
			..RenderSettings::default()
		};
		assert_eq!(settings(PresentMode::Fifo).pick_present_mode(&fifo_only), P::Fifo);
		assert_eq!(
			settings(PresentMode::Mailbox).pick_present_mode(&fifo_only),
			P::AutoNoVsync,
			"no tearing wanted, no waiting wanted"
		);
		assert_eq!(
			settings(PresentMode::FifoRelaxed).pick_present_mode(&fifo_only),
			P::AutoVsync
		);
		assert_eq!(
			settings(PresentMode::AutoNoVsync).pick_present_mode(&caps(&[], &[])),
			P::AutoNoVsync,
			"the automatic modes are always there"
		);
	}

	#[test]
	fn the_alpha_mode_is_one_the_surface_has() {
		use wgpu::CompositeAlphaMode as A;
		let settings = |alpha_mode| RenderSettings {
			alpha_mode,
			..RenderSettings::default()
		};
		let surface = caps(&[], &[A::Opaque, A::PostMultiplied]);
		assert_eq!(settings(AlphaMode::Opaque).pick_alpha_mode(&surface), A::Opaque);
		assert_eq!(
			settings(AlphaMode::Transparent).pick_alpha_mode(&surface),
			A::PostMultiplied
		);
		assert_eq!(settings(AlphaMode::Inherit).pick_alpha_mode(&surface), A::Auto);
		assert_eq!(settings(AlphaMode::Auto).pick_alpha_mode(&surface), A::Auto);
	}

	#[test]
	fn validation_can_be_turned_on_and_off() {
		let checks = wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG;
		let on = RenderSettings {
			validation: Some(true),
			..RenderSettings::default()
		};
		let off = RenderSettings {
			validation: Some(false),
			..RenderSettings::default()
		};
		// The environment of the test may turn them on again; without it, the
		// settings decide.
		if std::env::var_os("WGPU_VALIDATION").is_none() && std::env::var_os("WGPU_DEBUG").is_none() {
			assert!(on.instance_flags().contains(checks));
			assert!(!off.instance_flags().intersects(checks));
		}
	}

	#[test]
	fn what_waits_for_the_next_start_is_told_apart() {
		let base = RenderSettings::default();
		let surface_only = RenderSettings {
			present_mode: PresentMode::Immediate,
			alpha_mode: AlphaMode::Transparent,
			max_frame_latency: 3,
			..RenderSettings::default()
		};
		let device = RenderSettings {
			backend: Backend::Vulkan,
			..RenderSettings::default()
		};
		assert!(base.same_device(&surface_only));
		assert!(!base.same_device(&device));
	}

	#[test]
	fn the_settings_survive_being_saved() -> Result<(), toml::de::Error> {
		let settings = RenderSettings {
			power: PowerPreference::LowPower,
			validation: Some(false),
			present_mode: PresentMode::Mailbox,
			alpha_mode: AlphaMode::Transparent,
			..RenderSettings::default()
		};
		let text = toml::to_string(&settings).unwrap_or_default();
		assert!(text.contains("PresentMode = \"Mailbox\""), "{text}");
		assert_eq!(toml::from_str::<RenderSettings>(&text)?, settings);
		assert_eq!(
			toml::from_str::<RenderSettings>("AlphaMode = \"Transparent\"")?,
			RenderSettings {
				alpha_mode: AlphaMode::Transparent,
				..RenderSettings::default()
			},
			"what a file leaves out is the default"
		);
		Ok(())
	}
}
