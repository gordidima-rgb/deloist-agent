//! Заглушка Node.js для тестов управляемой установки.
//!
//! - `node.exe --version` печатает `v26.0.0` (версия заведомо новее 20);
//! - `node.exe <...>/npm-cli.js install … --prefix <dir> <package>@<version>` создаёт
//!   `node_modules/<package>/package.json` и `bin/agent.js` — как настоящий npm;
//! - `FAKE_NODE_LOG` — файл, куда дописывается строка JSON о каждом запуске;
//! - `FAKE_NODE_AGENT` — если задан, при запуске агентского скрипта запускает эту программу
//!   с тем же stdio (в тестах это `fake_acp.exe`, чтобы проверка входа прошла).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::json;

fn log(kind: &str, args: &[String]) {
    let Ok(path) = std::env::var("FAKE_NODE_LOG") else { return };
    let entry = json!({ "kind": kind, "pid": std::process::id(), "args": args });
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{entry}");
    }
}

fn package_name(spec: &str) -> String {
    let start = usize::from(spec.starts_with('@'));
    match spec[start..].find('@') {
        Some(i) => spec[..start + i].to_string(),
        None => spec.to_string(),
    }
}

fn install(args: &[String]) {
    let mut prefix = PathBuf::new();
    let mut spec = String::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--prefix" {
            prefix = PathBuf::from(args.get(i + 1).cloned().unwrap_or_default());
            i += 2;
            continue;
        }
        if !args[i].starts_with('-') {
            spec = args[i].clone();
        }
        i += 1;
    }
    let name = package_name(&spec);
    let short = name.rsplit('/').next().unwrap_or(&name).to_string();
    let dir = prefix.join("node_modules").join(Path::new(&name));
    let _ = std::fs::create_dir_all(dir.join("bin"));
    let manifest = json!({ "name": name, "version": "1.0.0", "bin": { short: "bin/agent.js" } });
    let _ = std::fs::write(dir.join("package.json"), manifest.to_string());
    let _ = std::fs::write(dir.join("bin").join("agent.js"), "// fake npm bin\n");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("v26.0.0");
        return;
    }
    if args.iter().any(|a| a.to_ascii_lowercase().ends_with("npm-cli.js")) {
        log("npm", &args);
        install(&args);
        return;
    }
    if args.iter().any(|a| a.to_ascii_lowercase().ends_with("npx-cli.js")) {
        log("npx", &args);
        return;
    }
    log("agent", &args);
    if let Ok(agent) = std::env::var("FAKE_NODE_AGENT") {
        if Path::new(&agent).is_file() {
            let _ = std::process::Command::new(agent).args(&args).status();
        }
    }
}
