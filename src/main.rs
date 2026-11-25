// Release builds are GUI-only, with no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod steam;
mod ui;
mod vdf;

fn main() {
    ui::run();
}
