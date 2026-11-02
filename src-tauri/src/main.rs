// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if luma_lib::run_file_search_worker() {
        return;
    }
    luma_lib::run()
}
