//! Where the browser runtime gets its project: the build `game.zenithbuild`
//! that lies next to the page (`Builder::web_runtime` puts the page there).
//! The whole file is downloaded and opened from memory.

use std::sync::Arc;

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{OpenOptions, Project};

const BUILD_URL: &str = "game.zenithbuild";

fn failed(what: &str, error: &JsValue) -> ZenithError {
	ZenithError::ProjectNotFound(format!("{BUILD_URL}: {what}: {error:?}"))
}

/// Downloads and opens the build next to the page.
///
/// # Errors
/// `ProjectNotFound` if the download fails, otherwise the errors of
/// `Project::open_archive_bytes` (an encrypted build needs a key, which the
/// web runtime does not have yet).
pub async fn open_project() -> ZResult<Project> {
	let window = web_sys::window().ok_or_else(|| ZenithError::ProjectNotFound("there is no window".into()))?;
	let response: web_sys::Response = JsFuture::from(window.fetch_with_str(BUILD_URL))
		.await
		.map_err(|e| failed("download", &e))?
		.dyn_into()
		.map_err(|e| failed("not a response", &e))?;
	if !response.ok() {
		return Err(ZenithError::ProjectNotFound(format!(
			"{BUILD_URL}: the server answered {}",
			response.status()
		)));
	}
	let buffer = JsFuture::from(response.array_buffer().map_err(|e| failed("body", &e))?)
		.await
		.map_err(|e| failed("body", &e))?;
	let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
	Project::open_archive_bytes(Arc::from(bytes), &OpenOptions::new())
}
