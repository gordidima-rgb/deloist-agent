//! Deloist-Setup.exe: загрузчик-установщик только для Windows (код в setup_impl.rs).
//! На других системах это пустая программа, чтобы `cargo build` собирал пакет целиком.

#![windows_subsystem = "windows"]

#[cfg(windows)]
include!("setup_impl.rs");

#[cfg(not(windows))]
fn main() {
    eprintln!("Deloist-Setup работает только в Windows.");
    std::process::exit(1);
}
