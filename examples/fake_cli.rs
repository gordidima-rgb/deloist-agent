//! Заглушка CLI-агента для тестов: читает промпт из stdin и печатает отчёт.
//! Копия под именем `claude.exe` умеет показывать версию и обновляться: версия лежит рядом в файле.

use std::io::Read;

fn version_file() -> std::path::PathBuf {
    std::env::current_exe().unwrap().with_file_name("version.txt")
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version") => {
            let version = std::fs::read_to_string(version_file()).unwrap_or_else(|_| "2.1.318".to_string());
            println!("{} (Fake Code)", version.trim());
            return;
        }
        Some("update") => {
            if std::env::current_exe().unwrap().with_file_name("broken.txt").exists() {
                eprintln!("обновление сломалось");
                std::process::exit(3);
            }
            std::fs::write(version_file(), "2.1.319").unwrap();
            return;
        }
        _ => {}
    }
    let mut data = String::new();
    std::io::stdin().read_to_string(&mut data).expect("промпт из stdin");
    println!("получено символов: {}", data.chars().count());
    println!("первая строка: {}", data.lines().next().unwrap_or_default());
    println!("есть слово «маяк»: {}", if data.contains("маяк") { "да" } else { "нет" });
}
