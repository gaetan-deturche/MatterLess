// Keeps the console window from appearing behind the app on Windows release
// builds; dev builds keep it so tracing output is visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    matterless_app_lib::run()
}
