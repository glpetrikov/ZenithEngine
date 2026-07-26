// TEMPORARILY DISABLED FOR DEBUGGING – restores console on Windows release
// builds so that .NET hosting diagnostics are visible when running from
// PowerShell. TODO: re-enable for final shipping build.
//
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() { zeroengine::run(); }
