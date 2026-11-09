//! File watching for the `Fs` backend.
//!
//! One recursive OS watcher on the project root (`notify` through
//! `notify-debouncer-full`, which also pairs rename events). The debouncer owns
//! the thread; its callback filters events and pushes them into a bounded queue
//! that the consumer drains without blocking. Everything the project does
//! itself (`write`, `move`, `remove`, `restore`) shows up here like any
//! external change, unmarked; duplicates are possible.
//!
//! # `Settings/Build.toml`
//!
//! Other tools keep their own sections in the same file, so unknown keys are
//! ignored.
//!
//! ```toml
//! [[watch]]
//! name = "rust"
//! patterns = ["**/*.rs"]     # globs relative to the project root, `/` separators
//! exclude = ["target/**"]    # optional
//! ```

use std::{
	collections::VecDeque,
	io,
	path::Path,
	sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
	time::{Duration, Instant},
};

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use notify::{
	EventKind, RecommendedWatcher, RecursiveMode,
	event::{ModifyKind, RenameMode},
};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use serde::Deserialize;
use zenith_error::{ZResult, ZenithError};

use crate::{
	fs::FsV1,
	layout::{ALWAYS_IGNORED_DIRS, BUILD_CONFIG_FILE, GITIGNORE, WATCH_DEBOUNCE_MS, WATCH_QUEUE_CAPACITY},
	zpath::ZPath,
};

/// A rule: files matching any of `patterns` and none of `exclude` are reported.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct WatchRule {
	pub name: String,
	pub patterns: Vec<String>,
	#[serde(default)]
	pub exclude: Vec<String>,
}

/// Where watch rules come from.
#[derive(Clone, Debug)]
pub enum WatchConfig {
	/// `[[watch]]` tables of `Settings/Build.toml`; no file, no rules, no
	/// events.
	File,
	/// Rules given in code, never written to disk.
	Manual(Vec<WatchRule>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
	Created,
	Modified,
	Removed,
	Renamed { from: ZPath },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
	pub kind: ChangeKind,
	pub path: ZPath,
	/// Names of the rules the path matched (at least one).
	pub rules: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectEvent {
	Change(FileChange),
	/// Events were lost; treat everything as changed.
	Rescan,
}

#[derive(Deserialize)]
struct BuildToml {
	#[serde(default)]
	watch: Vec<WatchRule>,
}

// --- queue --------------------------------------------------------------

struct State {
	events: VecDeque<ProjectEvent>,
	/// A `Rescan` is queued and not yet taken: nothing else is accepted until
	/// it is.
	rescan_pending: bool,
}

struct Queue {
	state: Mutex<State>,
	ready: Condvar,
	capacity: usize,
}

impl Queue {
	const fn new(capacity: usize) -> Self {
		Self {
			state: Mutex::new(State {
				events: VecDeque::new(),
				rescan_pending: false,
			}),
			ready: Condvar::new(),
			capacity,
		}
	}

	fn lock(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// Queues an event. When the queue is full the backlog is dropped and a
	/// single `Rescan` takes its place: the consumer is too slow to follow
	/// individual events anyway, and one marker is cheaper than a lie.
	fn push(&self, event: ProjectEvent) {
		let mut state = self.lock();
		if state.rescan_pending {
			return;
		}
		if matches!(event, ProjectEvent::Rescan) || state.events.len() >= self.capacity {
			state.events.clear();
			state.events.push_back(ProjectEvent::Rescan);
			state.rescan_pending = true;
		} else {
			state.events.push_back(event);
		}
		drop(state);
		self.ready.notify_all();
	}

	fn take(state: &mut State) -> Option<ProjectEvent> {
		let event = state.events.pop_front()?;
		if event == ProjectEvent::Rescan {
			state.rescan_pending = false;
		}
		Some(event)
	}
}

/// Receiving end of the event queue.
#[derive(Clone)]
pub struct EventReceiver {
	queue: Arc<Queue>,
}

impl EventReceiver {
	/// Next event, if one is waiting. Never blocks.
	#[must_use]
	pub fn try_recv(&self) -> Option<ProjectEvent> {
		Queue::take(&mut self.queue.lock())
	}

	/// Everything that is waiting right now.
	pub fn try_iter(&self) -> impl Iterator<Item = ProjectEvent> + '_ {
		std::iter::from_fn(|| self.try_recv())
	}

	/// Waits up to `timeout` for the next event.
	#[must_use]
	pub fn recv_timeout(&self, timeout: Duration) -> Option<ProjectEvent> {
		let deadline = Instant::now().checked_add(timeout)?;
		let mut state = self.queue.lock();
		loop {
			if let Some(event) = Queue::take(&mut state) {
				return Some(event);
			}
			let left = deadline.checked_duration_since(Instant::now())?;
			state = self
				.queue
				.ready
				.wait_timeout(state, left)
				.unwrap_or_else(PoisonError::into_inner)
				.0;
		}
	}
}

// --- rules --------------------------------------------------------------

struct CompiledRule {
	name: String,
	include: GlobSet,
	exclude: GlobSet,
}

fn glob_set(patterns: &[String], rule: &str) -> ZResult<GlobSet> {
	let mut builder = GlobSetBuilder::new();
	for pattern in patterns {
		let glob: Glob = GlobBuilder::new(pattern)
			.literal_separator(true)
			.build()
			.map_err(|e| ZenithError::InvalidSettings(format!("watch rule {rule:?}, pattern {pattern:?}: {e}")))?;
		builder.add(glob);
	}
	builder
		.build()
		.map_err(|e| ZenithError::InvalidSettings(format!("watch rule {rule:?}: {e}")))
}

fn compile(rules: &[WatchRule]) -> ZResult<Vec<CompiledRule>> {
	rules
		.iter()
		.map(|r| {
			Ok(CompiledRule {
				name: r.name.clone(),
				include: glob_set(&r.patterns, &r.name)?,
				exclude: glob_set(&r.exclude, &r.name)?,
			})
		})
		.collect()
}

fn matching_rules(rules: &[CompiledRule], rel: &str) -> Vec<String> {
	rules
		.iter()
		.filter(|r| r.include.is_match(rel) && !r.exclude.is_match(rel))
		.map(|r| r.name.clone())
		.collect()
}

// --- watcher ------------------------------------------------------------

/// Running watch. Dropping it stops the OS watcher and its thread.
pub struct Watcher {
	// Only held to keep the watch alive; `None` when there are no rules.
	_debouncer: Option<Debouncer<RecommendedWatcher, RecommendedCache>>,
	events: EventReceiver,
}

impl Watcher {
	/// Events that passed the rules.
	#[must_use]
	pub const fn events(&self) -> &EventReceiver {
		&self.events
	}
}

struct Context {
	project: FsV1,
	rules: Vec<CompiledRule>,
	queue: Arc<Queue>,
}

impl FsV1 {
	/// Starts watching the project folder.
	pub fn watch(&self, config: &WatchConfig) -> ZResult<Watcher> {
		self.watch_with_capacity(config, WATCH_QUEUE_CAPACITY)
	}

	pub(crate) fn watch_with_capacity(&self, config: &WatchConfig, capacity: usize) -> ZResult<Watcher> {
		let rules = match config {
			WatchConfig::Manual(rules) => rules.clone(),
			WatchConfig::File => match self.dir.read_to_string(BUILD_CONFIG_FILE) {
				Ok(text) => {
					toml::from_str::<BuildToml>(&text)
						.map_err(|e| ZenithError::InvalidSettings(format!("{BUILD_CONFIG_FILE}: {e}")))?
						.watch
				}
				Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
				Err(e) => return Err(e.into()),
			},
		};
		let queue = Arc::new(Queue::new(capacity.max(1)));
		let events = EventReceiver { queue: queue.clone() };
		if rules.is_empty() {
			return Ok(Watcher {
				_debouncer: None,
				events,
			});
		}

		// The block may be edited while we run; read it once up front.
		self.sync_gitignore_lenient();
		let context = Context {
			project: self.clone(),
			rules: compile(&rules)?,
			queue,
		};
		let mut debouncer = new_debouncer(
			Duration::from_millis(WATCH_DEBOUNCE_MS),
			None,
			move |result: DebounceEventResult| context.handle(result),
		)
		.map_err(|e| ZenithError::Watch(e.to_string()))?;
		debouncer
			.watch(&self.watch_root, RecursiveMode::Recursive)
			.map_err(|e| ZenithError::Watch(e.to_string()))?;
		Ok(Watcher {
			_debouncer: Some(debouncer),
			events,
		})
	}
}

enum Change {
	Created(String),
	Modified(String),
	Removed(String),
	Renamed { from: String, to: String },
}

impl Context {
	fn handle(&self, result: DebounceEventResult) {
		match result {
			Ok(events) => {
				for event in events {
					for change in self.changes(&event.event) {
						self.dispatch(change);
					}
				}
			}
			// Backend errors usually mean lost events (an overflowing OS
			// buffer).
			Err(_) => self.queue.push(ProjectEvent::Rescan),
		}
	}

	/// Project-relative `/` path of an absolute event path.
	fn relative(&self, path: &Path) -> Option<String> {
		let root = &self.project.watch_root;
		let rel = path.strip_prefix(root).ok().or_else(|| {
			// Some platforms report `\\?\`-prefixed paths for an unprefixed
			// root.
			let text = path.to_str()?.strip_prefix(r"\\?\")?;
			Path::new(text).strip_prefix(root).ok()
		})?;
		let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
		let joined = parts?.join("/");
		(!joined.is_empty()).then_some(joined)
	}

	fn changes(&self, event: &notify::Event) -> Vec<Change> {
		let rels: Vec<String> = event.paths.iter().filter_map(|p| self.relative(p)).collect();
		match &event.kind {
			// A half of a rename that was not paired: the file arrived or left.
			EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
				rels.into_iter().map(Change::Created).collect()
			}
			EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
				rels.into_iter().map(Change::Removed).collect()
			}
			EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => match <[String; 2]>::try_from(rels) {
				Ok([from, to]) => vec![Change::Renamed { from, to }],
				Err(_) => Vec::new(),
			},
			EventKind::Modify(_) => rels.into_iter().map(Change::Modified).collect(),
			_ => Vec::new(),
		}
	}

	fn is_ignored(&self, rel: &str) -> bool {
		let top = rel.split('/').next().unwrap_or(rel);
		if ALWAYS_IGNORED_DIRS.contains(&top) {
			return true;
		}
		self.project
			.managed
			.read()
			.unwrap_or_else(PoisonError::into_inner)
			.iter()
			.any(|dir| rel == dir || rel.strip_prefix(dir.as_str()).is_some_and(|rest| rest.starts_with('/')))
	}

	/// Removal of a folder the managed block lists means the entry is stale.
	fn covers_managed(&self, removed: &str) -> bool {
		self.project
			.managed
			.read()
			.unwrap_or_else(PoisonError::into_inner)
			.iter()
			.any(|dir| dir == removed || dir.strip_prefix(removed).is_some_and(|rest| rest.starts_with('/')))
	}

	fn dispatch(&self, change: Change) {
		// Internal bookkeeping comes before any filtering, so ignored folders
		// are still noticed when they disappear.
		let touched: Vec<&str> = match &change {
			Change::Created(p) | Change::Modified(p) | Change::Removed(p) => vec![p],
			Change::Renamed { from, to } => vec![from, to],
		};
		if touched.contains(&GITIGNORE) {
			self.project.sync_gitignore_lenient();
		}
		let removed = match &change {
			Change::Removed(p) => Some(p),
			Change::Renamed { from, .. } => Some(from),
			_ => None,
		};
		if removed.is_some_and(|p| self.covers_managed(p)) {
			self.project.sync_gitignore_lenient();
		}
		for rel in &touched {
			if !matches!(change, Change::Created(_)) {
				self.project.core.cache.invalidate_prefix(rel);
			}
		}

		let event = match change {
			Change::Created(p) => self.change(ChangeKindRaw::Created, &p),
			Change::Modified(p) => self.change(ChangeKindRaw::Modified, &p),
			Change::Removed(p) => self.change(ChangeKindRaw::Removed, &p),
			Change::Renamed { from, to } => match (self.is_ignored(&from), self.is_ignored(&to)) {
				// Moved out of an ignored folder (a restore): it appears.
				(true, false) => self.change(ChangeKindRaw::Created, &to),
				// Moved into an ignored folder (a remove): it disappears.
				(false, true) => self.change(ChangeKindRaw::Removed, &from),
				(true, true) => None,
				(false, false) => self.project.reverse(&from).map_or_else(
					|| self.change(ChangeKindRaw::Created, &to),
					|from_path| self.change(ChangeKindRaw::Renamed(from_path), &to),
				),
			},
		};
		if let Some(event) = event {
			self.queue.push(event);
		}
	}

	fn change(&self, kind: ChangeKindRaw, rel: &str) -> Option<ProjectEvent> {
		if self.is_ignored(rel) {
			return None;
		}
		let rules = matching_rules(&self.rules, rel);
		if rules.is_empty() {
			return None;
		}
		// Paths under no scheme (the manifest, `.gitignore`, ...) are not
		// reported.
		let path = self.project.reverse(rel)?;
		let kind = match kind {
			ChangeKindRaw::Created => ChangeKind::Created,
			ChangeKindRaw::Modified => ChangeKind::Modified,
			ChangeKindRaw::Removed => ChangeKind::Removed,
			ChangeKindRaw::Renamed(from) => ChangeKind::Renamed { from },
		};
		Some(ProjectEvent::Change(FileChange { kind, path, rules }))
	}
}

enum ChangeKindRaw {
	Created,
	Modified,
	Removed,
	Renamed(ZPath),
}

#[cfg(test)]
mod tests {
	use super::*;

	fn change(n: u32) -> ProjectEvent {
		ProjectEvent::Change(FileChange {
			kind: ChangeKind::Created,
			path: ZPath::from_parts("assets", &format!("f{n}")).unwrap_or_else(|_| unreachable!()),
			rules: vec!["r".into()],
		})
	}

	#[test]
	fn overflow_collapses_into_one_rescan() {
		let queue = Arc::new(Queue::new(3));
		let rx = EventReceiver { queue: queue.clone() };
		for n in 0..3 {
			queue.push(change(n));
		}
		assert_eq!(rx.try_iter().count(), 3, "a full queue delivers what it holds");

		for n in 0..4 {
			queue.push(change(n));
		}
		queue.push(change(99));
		assert_eq!(
			rx.try_recv(),
			Some(ProjectEvent::Rescan),
			"the backlog was replaced by a Rescan"
		);
		assert_eq!(rx.try_recv(), None, "and nothing else is queued");
		queue.push(change(7));
		assert_eq!(
			rx.try_recv(),
			Some(change(7)),
			"events flow again once the Rescan was taken"
		);
	}

	#[test]
	fn nothing_is_accepted_while_a_rescan_waits() {
		let queue = Arc::new(Queue::new(2));
		let rx = EventReceiver { queue: queue.clone() };
		queue.push(ProjectEvent::Rescan);
		queue.push(change(1));
		queue.push(ProjectEvent::Rescan);
		assert_eq!(
			rx.try_iter().collect::<Vec<_>>(),
			vec![ProjectEvent::Rescan],
			"exactly one Rescan"
		);
	}

	#[test]
	fn recv_timeout_waits_and_times_out() {
		let queue = Arc::new(Queue::new(8));
		let rx = EventReceiver { queue: queue.clone() };
		assert_eq!(rx.recv_timeout(Duration::from_millis(20)), None, "times out when empty");
		let pusher = {
			std::thread::spawn(move || {
				std::thread::sleep(Duration::from_millis(30));
				queue.push(change(5));
			})
		};
		assert_eq!(
			rx.recv_timeout(Duration::from_secs(5)),
			Some(change(5)),
			"wakes up for a new event"
		);
		assert!(pusher.join().is_ok(), "pusher finished");
	}

	#[test]
	fn rule_matching() -> ZResult<()> {
		let rules = compile(&[
			WatchRule {
				name: "rust".into(),
				patterns: vec!["**/*.rs".into()],
				exclude: vec!["gen/**".into()],
			},
			WatchRule {
				name: "all".into(),
				patterns: vec!["Assets/*".into()],
				exclude: vec![],
			},
		])?;
		assert_eq!(
			matching_rules(&rules, "a.rs"),
			vec!["rust"],
			"** matches zero directories"
		);
		assert_eq!(matching_rules(&rules, "src/deep/a.rs"), vec!["rust"], "and many");
		assert!(matching_rules(&rules, "gen/a.rs").is_empty(), "exclude wins");
		assert_eq!(
			matching_rules(&rules, "Assets/a.rs"),
			vec!["rust", "all"],
			"several rules can match"
		);
		assert_eq!(
			matching_rules(&rules, "Assets/sub/a.png"),
			Vec::<String>::new(),
			"`*` does not cross `/`"
		);
		assert!(
			compile(&[WatchRule {
				name: "bad".into(),
				patterns: vec!["[".into()],
				exclude: vec![]
			}])
			.is_err(),
			"bad glob"
		);
		Ok(())
	}

	#[test]
	fn build_toml_ignores_foreign_sections() -> ZResult<()> {
		let parsed: BuildToml = toml::from_str(
			"[scripts]\ncompiler = \"x\"\n\n[[watch]]\nname = \"rust\"\npatterns = [\"**/*.rs\"]\nunknown = 1\n",
		)
		.map_err(|e| ZenithError::Toml(e.to_string()))?;
		assert_eq!(parsed.watch.len(), 1, "one rule");
		assert!(
			parsed.watch.first().is_some_and(|r| r.exclude.is_empty()),
			"exclude is optional"
		);
		Ok(())
	}
}
