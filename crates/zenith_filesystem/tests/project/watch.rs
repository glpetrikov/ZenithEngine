//! These tests use the real OS watcher, so they cannot know when an event
//! arrives. They never wait a fixed time for something to happen or not to
//! happen: to prove that something stayed silent they write a *marker* file
//! after it and wait for the marker's event, since events come in the order of
//! the changes. The only waits are the upper bounds in [`WAIT`], which a
//! passing test does not use up.

use std::{
	path::Path,
	time::{Duration, Instant},
};

use zenith_filesystem::{ChangeKind, EventReceiver, FileChange, Project, ProjectEvent, WatchConfig, WatchRule};

use crate::common::{TestResult, new_project, z};

/// How long a test waits for an event it expects before it gives up.
const WAIT: Duration = Duration::from_secs(15);

fn rule(name: &str, patterns: &[&str]) -> WatchRule {
	WatchRule {
		name: name.into(),
		patterns: patterns.iter().map(|p| (*p).to_string()).collect(),
		exclude: Vec::new(),
	}
}

/// Every event up to and including the first change that `wanted` accepts.
/// `None` if none arrives within [`WAIT`].
fn collect_until(rx: &EventReceiver, wanted: impl Fn(&FileChange) -> bool) -> Option<Vec<ProjectEvent>> {
	let deadline = Instant::now() + WAIT;
	let mut seen = Vec::new();
	while let Some(left) = deadline.checked_duration_since(Instant::now()) {
		let Some(event) = rx.recv_timeout(left) else {
			continue;
		};
		let hit = matches!(&event, ProjectEvent::Change(change) if wanted(change));
		seen.push(event);
		if hit {
			return Some(seen);
		}
	}
	None
}

/// Waits until `wanted` accepts an event; returns it.
fn wait_for(rx: &EventReceiver, wanted: impl Fn(&FileChange) -> bool) -> Option<FileChange> {
	collect_until(rx, &wanted)?
		.into_iter()
		.rev()
		.find_map(|event| match event {
			ProjectEvent::Change(change) if wanted(&change) => Some(change),
			_ => None,
		})
}

/// Writes `Assets/<name>` and waits for its event. Whatever was written before
/// it has been delivered (or dropped) by then.
fn marker(root: &Path, rx: &EventReceiver, name: &str) -> Result<Vec<ProjectEvent>, Box<dyn std::error::Error>> {
	std::fs::write(root.join("Assets").join(name), b"marker")?;
	let wanted = format!("Assets://{name}");
	collect_until(rx, |c| c.path.to_string() == wanted).ok_or_else(|| format!("no event for the marker {name}").into())
}

fn paths(events: &[ProjectEvent]) -> Vec<String> {
	events
		.iter()
		.filter_map(|e| match e {
			ProjectEvent::Change(c) => Some(c.path.to_string()),
			ProjectEvent::Rescan => None,
		})
		.collect()
}

#[test]
fn external_changes_are_reported_with_their_rules() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let watcher = project.watch(&WatchConfig::Manual(vec![
		rule("text", &["**/*.txt"]),
		rule("everything", &["Assets/**"]),
	]))?;
	let rx = watcher.events();

	std::fs::write(root.join("Assets/a.txt"), b"1")?;
	let created = wait_for(rx, |c| c.path.to_string() == "Assets://a.txt").ok_or("no event for a.txt")?;
	assert!(
		matches!(created.kind, ChangeKind::Created | ChangeKind::Modified),
		"a new file: {:?}",
		created.kind
	);
	assert_eq!(
		created.rules,
		vec!["text", "everything"],
		"every matching rule is listed"
	);

	std::fs::write(root.join("Assets/b.bin"), b"1")?;
	let other = wait_for(rx, |c| c.path.to_string() == "Assets://b.bin").ok_or("no event for b.bin")?;
	assert_eq!(other.rules, vec!["everything"], "only the matching rule");

	std::fs::write(root.join("Assets/Worlds/w.txt"), b"1")?;
	let world = wait_for(rx, |c| c.path.to_string().ends_with("w.txt")).ok_or("no event for w.txt")?;
	assert_eq!(
		world.path.to_string(),
		"Worlds://w.txt",
		"the scheme with the longest root is used"
	);

	std::fs::remove_file(root.join("Assets/a.txt"))?;
	assert!(
		wait_for(rx, |c| c.kind == ChangeKind::Removed
			&& c.path.to_string() == "Assets://a.txt")
		.is_some(),
		"removal"
	);
	Ok(())
}

#[test]
fn ignored_places_stay_silent() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let watcher = project.watch(&WatchConfig::Manual(vec![rule("all", &["**/*"])]))?;
	let rx = watcher.events();
	project.ensure_ignored_dir(root.join("Out"))?;

	std::fs::write(root.join("Temp/t.txt"), b"1")?;
	std::fs::write(root.join("Trash/t.txt"), b"1")?;
	std::fs::create_dir_all(root.join("Build"))?;
	std::fs::write(root.join("Build/b.txt"), b"1")?;
	std::fs::write(root.join("Out/o.txt"), b"1")?;

	// The marker comes after all of them; once its event is here, theirs would
	// have been too.
	let events = marker(&root, rx, "visible.txt")?;
	let seen = paths(&events);
	for ignored in ["Temp://", "Trash://", "Build", "Out"] {
		assert!(
			!seen.iter().any(|p| p.starts_with(ignored)),
			"{ignored} must stay silent: {seen:?}"
		);
	}
	let later = paths(&rx.try_iter().collect::<Vec<_>>());
	assert!(
		later.iter().all(|p| p.starts_with("Assets://")),
		"and nothing from them arrives afterwards: {later:?}"
	);
	Ok(())
}

#[test]
fn removing_through_the_project_reports_removed_and_restore_reports_created() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://x.txt")?, b"1")?;
	let watcher = project.watch(&WatchConfig::Manual(vec![rule("all", &["Assets/**"])]))?;
	let rx = watcher.events();
	marker(&root, rx, "ready.txt")?;

	let id = project.remove(&z("Assets://x.txt")?)?;
	assert!(
		wait_for(rx, |c| c.kind == ChangeKind::Removed
			&& c.path.to_string() == "Assets://x.txt")
		.is_some(),
		"a move into the ignored trash is a Removed"
	);
	project.restore(&id)?;
	assert!(
		wait_for(rx, |c| matches!(c.kind, ChangeKind::Created)
			&& c.path.to_string() == "Assets://x.txt")
		.is_some(),
		"a move out of the ignored trash is a Created"
	);
	Ok(())
}

#[test]
fn renames_are_paired_or_split() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://old.txt")?, b"1")?;
	let watcher = project.watch(&WatchConfig::Manual(vec![rule("all", &["Assets/**"])]))?;
	let rx = watcher.events();
	marker(&root, rx, "ready.txt")?;

	project.move_to(&z("Assets://old.txt")?, &z("Assets://new.txt")?)?;
	let events = marker(&root, rx, "after.txt")?;
	let renamed = events.iter().any(|e| {
		matches!(e, ProjectEvent::Change(c) if c.path.to_string() == "Assets://new.txt"
		&& matches!(&c.kind, ChangeKind::Renamed { from } if from.to_string() == "Assets://old.txt"))
	});
	let split = paths(&events).iter().any(|p| p == "Assets://old.txt")
		&& paths(&events).iter().any(|p| p == "Assets://new.txt");
	assert!(
		renamed || split,
		"either Renamed {{ from }} or Removed + Created, got {events:?}"
	);
	Ok(())
}

#[test]
fn exclude_patterns_win() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let mut r = rule("text", &["**/*.txt"]);
	r.exclude = vec!["Assets/skip/**".into()];
	let watcher = project.watch(&WatchConfig::Manual(vec![r]))?;
	std::fs::create_dir_all(root.join("Assets/skip"))?;
	std::fs::write(root.join("Assets/skip/s.txt"), b"1")?;
	let events = marker(&root, watcher.events(), "keep.txt")?;
	let seen = paths(&events);
	assert!(seen.iter().any(|p| p == "Assets://keep.txt"), "kept: {seen:?}");
	assert!(!seen.iter().any(|p| p.contains("skip")), "excluded: {seen:?}");
	Ok(())
}

#[test]
fn rules_come_from_build_toml_and_no_file_means_no_events() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	{
		// Without rules no OS watcher is started at all, so there is nothing
		// that could still deliver an event; a short look is enough.
		let watcher = project.watch(&WatchConfig::File)?;
		std::fs::write(root.join("Assets/a.txt"), b"1")?;
		assert!(
			watcher.events().recv_timeout(Duration::from_millis(300)).is_none(),
			"no Build.toml, no rules, no events"
		);
		assert!(
			!root.join("Settings/Build.toml").exists(),
			"and the file is not created"
		);
	}

	std::fs::write(
		root.join("Settings/Build.toml"),
		"[scripts]\ncompiler = \"x\"\n\n[[watch]]\nname = \"txt\"\npatterns = [\"Assets/*.txt\"]\nfuture_key = true\n",
	)?;
	let watcher = project.watch(&WatchConfig::File)?;
	std::fs::write(root.join("Assets/b.txt"), b"1")?;
	let change = wait_for(watcher.events(), |c| c.path.to_string() == "Assets://b.txt")
		.ok_or("no event from the file's rules")?;
	assert_eq!(change.rules, vec!["txt"], "rule name from Build.toml");

	std::fs::write(
		root.join("Settings/Build.toml"),
		"[[watch]]\nname = \"bad\"\npatterns = [\"[\"]\n",
	)?;
	assert!(
		project.watch(&WatchConfig::File).is_err(),
		"an invalid glob is reported"
	);
	Ok(())
}

#[test]
fn external_changes_invalidate_the_cache() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let path = z("Assets://data.txt")?;
	std::fs::write(root.join("Assets/data.txt"), b"first")?;
	let watcher = project.watch(&WatchConfig::Manual(vec![rule("all", &["Assets/**"])]))?;
	for _ in 0..3 {
		project.read_bytes(&path)?;
	}
	std::fs::write(root.join("Assets/data.txt"), b"second!")?;
	wait_for(watcher.events(), |c| c.path.to_string() == "Assets://data.txt").ok_or("no event")?;
	assert_eq!(&*project.read_bytes(&path)?, b"second!", "the change is visible");
	Ok(())
}

#[test]
fn dropping_the_watcher_stops_it() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let watcher = project.watch(&WatchConfig::Manual(vec![rule("all", &["**/*"])]))?;
	let rx = watcher.events().clone();
	marker(&root, &rx, "alive.txt")?;
	drop(watcher);
	std::fs::write(root.join("Assets/late.txt"), b"1")?;
	// The debounce window is 200 ms; waiting a good deal longer than that for
	// something that must not come is the one wait that cannot be avoided.
	let late = rx.recv_timeout(Duration::from_millis(800));
	assert!(
		!matches!(&late, Some(ProjectEvent::Change(c)) if c.path.to_string() == "Assets://late.txt"),
		"nothing arrives after the watcher is gone: {late:?}"
	);
	let _: &Project = &project;
	Ok(())
}
