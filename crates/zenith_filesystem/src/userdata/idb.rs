//! A store in IndexedDB, for a browser.
//!
//! IndexedDB can only be asked asynchronously, and a
//! [`Project`](crate::Project) is read and written synchronously. So the whole
//! store is read into memory when it is opened (it is small: settings and
//! saves), reads and writes go to that copy at once, and every write is also
//! started in the background, in parallel with the game. A write that has not
//! finished is remembered; when the page is hidden or closed they are all
//! started again on the spot, since nothing can be awaited then.

// `IndexedDB` is the name of a browser feature, not code. Everything in a
// browser runs on one thread, so its futures are not (and need not be) `Send`.
#![allow(clippy::doc_markdown, clippy::future_not_send)]

use std::{
	cell::RefCell,
	collections::{BTreeMap, HashMap},
	sync::{
		Arc, Mutex, PoisonError,
		atomic::{AtomicU64, Ordering},
	},
};

use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{IdbDatabase, IdbRequest, IdbTransaction, IdbTransactionMode};
use zenith_error::{ZResult, ZenithError};

use super::{MemStore, UserStore};
use crate::types::{DirEntry, Metadata};

/// The name of the object store (the table) that holds the files.
const STORE: &str = "files";

thread_local! {
	/// The open databases, by name. Browser objects cannot be shared between
	/// threads (there is one), so they are kept here and not in the store.
	static CONNECTIONS: RefCell<HashMap<String, IdbDatabase>> = RefCell::new(HashMap::new());
}

/// Writes that are not confirmed yet: the newest value of a path (`None` is a
/// deletion) and the number of the write, so that a confirmation of an older
/// write does not clear a newer one.
type Dirty = Arc<Mutex<BTreeMap<String, (u64, Option<Vec<u8>>)>>>;

fn lock(dirty: &Dirty) -> std::sync::MutexGuard<'_, BTreeMap<String, (u64, Option<Vec<u8>>)>> {
	dirty.lock().unwrap_or_else(PoisonError::into_inner)
}

fn js_error(what: &str, error: &JsValue) -> ZenithError {
	ZenithError::Io(std::io::Error::other(format!("IndexedDB {what}: {error:?}")))
}

/// A promise that comes true with the result of the request.
fn request_promise(request: &IdbRequest) -> Promise {
	let request = request.clone();
	Promise::new(&mut move |resolve, reject| {
		let found = request.clone();
		let on_success = Closure::once_into_js(move || {
			let result = found.result().unwrap_or(JsValue::UNDEFINED);
			let _ = resolve.call1(&JsValue::NULL, &result);
		});
		let failed = request.clone();
		let on_error = Closure::once_into_js(move || {
			let reason = failed
				.error()
				.ok()
				.flatten()
				.map_or_else(|| JsValue::from_str("the request failed"), JsValue::from);
			let _ = reject.call1(&JsValue::NULL, &reason);
		});
		request.set_onsuccess(Some(on_success.unchecked_ref()));
		request.set_onerror(Some(on_error.unchecked_ref()));
	})
}

/// A promise that comes true once the transaction is safely stored.
fn transaction_done(transaction: &IdbTransaction) -> Promise {
	let transaction = transaction.clone();
	Promise::new(&mut move |resolve, reject| {
		let on_complete = Closure::once_into_js(move || {
			let _ = resolve.call0(&JsValue::NULL);
		});
		let on_error = Closure::once_into_js({
			let reject = reject.clone();
			move || {
				let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("the transaction failed"));
			}
		});
		let on_abort = Closure::once_into_js(move || {
			let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("the transaction was cancelled"));
		});
		transaction.set_oncomplete(Some(on_complete.unchecked_ref()));
		transaction.set_onerror(Some(on_error.unchecked_ref()));
		transaction.set_onabort(Some(on_abort.unchecked_ref()));
	})
}

fn connection(database: &str) -> ZResult<IdbDatabase> {
	CONNECTIONS
		.with(|all| all.borrow().get(database).cloned())
		.ok_or_else(|| ZenithError::Unsupported(format!("the IndexedDB database {database:?} is not open")))
}

/// Starts a write or a deletion and returns the promise of its end.
fn start_write(database: &str, path: &str, value: Option<&[u8]>) -> ZResult<Promise> {
	let db = connection(database)?;
	let transaction = db
		.transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
		.map_err(|e| js_error("transaction", &e))?;
	let store = transaction
		.object_store(STORE)
		.map_err(|e| js_error("object store", &e))?;
	let key = JsValue::from_str(path);
	value
		.map_or_else(
			|| store.delete(&key),
			|bytes| store.put_with_key(&Uint8Array::from(bytes), &key),
		)
		.map_err(|e| js_error("write", &e))?;
	Ok(transaction_done(&transaction))
}

/// Writes what is remembered for `path`, in the background.
async fn persist(database: String, path: String, dirty: Dirty) {
	let Some((version, value)) = lock(&dirty).get(&path).cloned() else {
		return;
	};
	let done = match start_write(&database, &path, value.as_deref()) {
		Ok(promise) => JsFuture::from(promise)
			.await
			.map(|_| ())
			.map_err(|e| js_error("write", &e)),
		Err(e) => Err(e),
	};
	match done {
		Ok(()) => {
			let mut pending = lock(&dirty);
			if pending.get(&path).is_some_and(|(newest, _)| *newest == version) {
				pending.remove(&path);
			}
		}
		// It stays remembered: the next write of the file, or the page being
		// hidden, tries again.
		Err(e) => zenith_log::warn!("{e}"),
	}
}

/// Starts every unconfirmed write now, without waiting. For a page that is
/// going away: nothing can be awaited, but a request that was started usually
/// still finishes.
fn start_all(database: &str, dirty: &Dirty) {
	for (path, (_, value)) in lock(dirty).iter() {
		let _ = start_write(database, path, value.as_deref());
	}
}

/// Files kept in IndexedDB. See the [module](self) for how.
#[derive(Debug)]
pub struct IdbStore {
	database: String,
	memory: Arc<MemStore>,
	dirty: Dirty,
	writes: Arc<AtomicU64>,
}

impl IdbStore {
	/// Opens (and creates, the first time) the database called `database` and
	/// reads everything in it. Name it after the game, so that two games on
	/// one site do not share files.
	///
	/// # Errors
	/// `Unsupported` if the browser has no IndexedDB (a private window may
	/// not), an I/O error if it cannot be read.
	pub async fn open(database: &str) -> ZResult<Self> {
		let window =
			web_sys::window().ok_or_else(|| ZenithError::Unsupported("there is no window to keep data in".into()))?;
		let factory = window
			.indexed_db()
			.map_err(|e| js_error("access", &e))?
			.ok_or_else(|| ZenithError::Unsupported("this browser has no IndexedDB".into()))?;
		let request = factory.open_with_u32(database, 1).map_err(|e| js_error("open", &e))?;
		let creating = request.clone();
		let on_upgrade = Closure::once_into_js(move || {
			if let Ok(result) = creating.result()
				&& let Ok(db) = result.dyn_into::<IdbDatabase>()
			{
				let _ = db.create_object_store(STORE);
			}
		});
		request.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));
		let opened = JsFuture::from(request_promise(&request))
			.await
			.map_err(|e| js_error("open", &e))?;
		let db: IdbDatabase = opened
			.dyn_into()
			.map_err(|_| ZenithError::Unsupported("IndexedDB did not give a database".into()))?;

		let memory = Arc::new(MemStore::new());
		Self::load_all(&db, &memory).await?;
		CONNECTIONS.with(|all| all.borrow_mut().insert(database.to_string(), db));

		let dirty: Dirty = Arc::new(Mutex::new(BTreeMap::new()));
		Self::start_on_page_hide(database, &dirty, &window);
		Ok(Self {
			database: database.to_string(),
			memory,
			dirty,
			writes: Arc::new(AtomicU64::new(0)),
		})
	}

	async fn load_all(db: &IdbDatabase, memory: &MemStore) -> ZResult<()> {
		let transaction = db.transaction_with_str(STORE).map_err(|e| js_error("read", &e))?;
		let store = transaction
			.object_store(STORE)
			.map_err(|e| js_error("object store", &e))?;
		let keys = store.get_all_keys().map_err(|e| js_error("read", &e))?;
		let values = store.get_all().map_err(|e| js_error("read", &e))?;
		let keys: Array = JsFuture::from(request_promise(&keys))
			.await
			.map_err(|e| js_error("read", &e))?
			.dyn_into()
			.map_err(|_| ZenithError::Unsupported("IndexedDB did not give a list".into()))?;
		let values: Array = JsFuture::from(request_promise(&values))
			.await
			.map_err(|e| js_error("read", &e))?
			.dyn_into()
			.map_err(|_| ZenithError::Unsupported("IndexedDB did not give a list".into()))?;
		for (key, value) in keys.iter().zip(values.iter()) {
			if let Some(path) = key.as_string() {
				memory.insert_raw(&path, Uint8Array::new(&value).to_vec());
			}
		}
		Ok(())
	}

	/// When the page is hidden or closed, every write not confirmed yet is
	/// started again at once.
	fn start_on_page_hide(database: &str, dirty: &Dirty, window: &web_sys::Window) {
		let (name, pending) = (database.to_string(), Arc::clone(dirty));
		let handler = Closure::<dyn FnMut()>::new(move || start_all(&name, &pending));
		for event in ["pagehide", "visibilitychange"] {
			let _ = window.add_event_listener_with_callback(event, handler.as_ref().unchecked_ref());
		}
		// It lives as long as the page does.
		handler.forget();
	}

	fn remember(&self, path: &str, value: Option<Vec<u8>>) {
		let version = self.writes.fetch_add(1, Ordering::SeqCst);
		lock(&self.dirty).insert(path.to_string(), (version, value));
		spawn_local(persist(
			self.database.clone(),
			path.to_string(),
			Arc::clone(&self.dirty),
		));
	}

	/// Waits until every write is safe in IndexedDB. (`UserStore::flush` can
	/// only start them: a synchronous call cannot wait.)
	///
	/// # Errors
	/// An I/O error if a write fails.
	pub async fn flush_async(&self) -> ZResult<()> {
		let paths: Vec<String> = lock(&self.dirty).keys().cloned().collect();
		for path in paths {
			persist(self.database.clone(), path, Arc::clone(&self.dirty)).await;
		}
		if lock(&self.dirty).is_empty() {
			Ok(())
		} else {
			Err(ZenithError::Io(std::io::Error::other(
				"some data could not be written to IndexedDB",
			)))
		}
	}
}

impl UserStore for IdbStore {
	fn read(&self, path: &str) -> ZResult<Option<Vec<u8>>> {
		self.memory.read(path)
	}

	fn write(&self, path: &str, data: &[u8]) -> ZResult<()> {
		self.memory.write(path, data)?;
		self.remember(path, Some(data.to_vec()));
		Ok(())
	}

	fn remove(&self, path: &str) -> ZResult<bool> {
		let gone = self.memory.paths_under(path);
		let removed = self.memory.remove(path)?;
		for file in gone {
			self.remember(&file, None);
		}
		Ok(removed)
	}

	fn metadata(&self, path: &str) -> ZResult<Option<Metadata>> {
		self.memory.metadata(path)
	}

	fn list(&self, path: &str) -> ZResult<Vec<DirEntry>> {
		self.memory.list(path)
	}

	/// Starts the writes that are not confirmed yet; they finish in the
	/// background (see [`IdbStore::flush_async`] to wait).
	fn flush(&self) -> ZResult<()> {
		start_all(&self.database, &self.dirty);
		Ok(())
	}
}
