use std::{env, process};

fn main() {
	if let Err(error) = run() {
		eprintln!("{error}");
		process::exit(1);
	}
}

fn run() -> zepack::Result<()> {
	let args = env::args().collect::<Vec<_>>();
	let Some(command) = args.get(1).map(String::as_str) else {
		print_usage(&args[0]);
		return Ok(());
	};

	match command {
		"pack" if args.len() == 4 => zepack::pack_directory(&args[2], &args[3]),
		"unpack" if args.len() == 4 => zepack::unpack_file(&args[2], &args[3]),
		_ => {
			print_usage(&args[0]);
			Ok(())
		}
	}
}

fn print_usage(program: &str) {
	eprintln!("Usage:");
	eprintln!("  {program} pack <input_dir> <output_file>");
	eprintln!("  {program} unpack <input_file> <output_dir>");
}
