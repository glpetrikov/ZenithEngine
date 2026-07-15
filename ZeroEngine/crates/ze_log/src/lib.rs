use std::sync::OnceLock;

pub use tracing::{debug, debug_span, error, error_span, info, info_span, trace, trace_span, warn, warn_span};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

static EDITOR_EVENT_COLLECTOR: OnceLock<egui_tracing::EventCollector> = OnceLock::new();

pub fn init() {
	let filter = log_filter();
	let subscriber = tracing_subscriber::registry().with(filter).with(fmt_layer());

	if let Err(e) = subscriber.try_init() {
		eprintln!("Failed to initialize ZeroEngine Logger: {e}");
	}
}

pub fn init_editor(ctx: &egui::Context) {
	let collector = egui_tracing::EventCollector::default();
	let _ = EDITOR_EVENT_COLLECTOR.set(collector.clone());

	let subscriber = tracing_subscriber::registry()
		.with(log_filter())
		.with(fmt_layer())
		.with(collector);

	if let Err(e) = subscriber.try_init() {
		eprintln!("Failed to initialize ZeroEngine editor logger: {e}");
	}

	ctx.request_repaint();
}

pub fn show_editor_console(ui: &mut egui::Ui) {
	if let Some(collector) = EDITOR_EVENT_COLLECTOR.get() {
		ui.add(egui_tracing::Logs::new(collector.clone()));
	} else {
		ui.label("Editor logger is not initialized.");
	}
}

fn log_filter() -> EnvFilter {
	EnvFilter::builder()
		.with_default_directive(tracing::Level::INFO.into())
		.from_env_lossy()
		.add_directive("calloop=off".parse().unwrap_or_else(|e| {
			eprintln!("Invalid log directive: {e}");
			std::process::exit(1);
		}))
		.add_directive("winit=warn".parse().unwrap_or_else(|e| {
			eprintln!("Invalid log directive: {e}");
			std::process::exit(1);
		}))
		.add_directive("wgpu_hal=warn".parse().unwrap_or_else(|e| {
			eprintln!("Invalid log directive: {e}");
			std::process::exit(1);
		}))
		.add_directive("sctk=error".parse().unwrap_or_else(|e| {
			eprintln!("Invalid log directive: {e}");
			std::process::exit(1);
		}))
		.add_directive("sctk_adwaita=error".parse().unwrap_or_else(|e| {
			eprintln!("Invalid log directive: {e}");
			std::process::exit(1);
		}))
		.add_directive("egui=warn".parse().unwrap_or_else(|e| {
			eprintln!("Invalid log directive: {e}");
			std::process::exit(1);
		}))
}

fn fmt_layer<S>() -> impl tracing_subscriber::Layer<S>
where
	S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
	fmt::layer()
		.with_target(true)
		.with_file(true)
		.with_line_number(true)
		.with_thread_names(true)
		.with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
		.with_timer(fmt::time::time())
		.compact()
}
