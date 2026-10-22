// Release builds are GUI-subsystem (no console window); debug builds keep the console for engine diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    facebridge_app::run();
}
