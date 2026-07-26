# ZeroEngine

Main executable crate for the ZeroEngine workspace.

This crate is responsible for starting the engine runtime: it initializes logging, creates the application event loop, sets up shutdown handling, and runs the main app instance.

Most engine functionality lives in workspace crates; this crate acts as the entry point that wires everything together.
