// Prevents an extra console window on Windows release builds; harmless on Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    droidnative_app_lib::run()
}
