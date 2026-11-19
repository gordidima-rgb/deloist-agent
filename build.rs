//! Подготовка сборки под Windows без Visual Studio и MSYS2:
//! импорт-библиотеки делаем встроенным в Rust lld. Загрузчик WebView2 живёт в самом exe (src/webview_loader.rs).

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=link");
    println!("cargo:rerun-if-changed=web/icons");
    println!("cargo:rerun-if-changed=web/ui");
    println!("cargo:rerun-if-changed=catalog");
    // Несуществующий путь Cargo считает изменённым на каждой сборке: без папки следим за её родителем
    // (аудит 19.11.2026: лишние ~5,5 мин на повторной сборке без правок).
    let whats_new = if Path::new("docs/images/whats-new").is_dir() { "docs/images/whats-new" } else { "docs/images" };
    println!("cargo:rerun-if-changed={whats_new}");
    println!("cargo:rerun-if-changed=private");
    println!("cargo:rustc-check-cfg=cfg(deloist_private)");
    if Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("private").is_dir() {
        println!("cargo:rustc-cfg=deloist_private");
    }
    write_icon_table();
    write_ui_table();
    write_catalog_table();
    write_whats_new_images();
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    build_import_libs(&manifest.join("link"), &out_dir);
    build_icon_obj(&manifest, &out_dir);
}

/// Значки приложений (web/icons/apps) встраиваются в exe без правки server.rs на каждый файл.
fn write_icon_table() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("web").join("icons").join("apps");
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    files.sort();
    let mut table = String::from("&[\n");
    for file in &files {
        let relative = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let kind = match file.extension().and_then(|e| e.to_str()) {
            Some("svg") => "image/svg+xml",
            Some("json") => "application/json",
            _ => continue,
        };
        table.push_str(&format!("    ({:?}, {:?}, include_bytes!({:?}) as &[u8]),\n", format!("/icons/apps/{relative}"), kind, file.display().to_string()));
    }
    table.push_str("]\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app_icons.rs");
    std::fs::write(out, table).expect("запись таблицы значков");
}

/// Собранные Svelte-компоненты (web/ui/*) встраиваются в exe без правки server.rs на каждый файл.
/// Файлы с точки в начале (.source-hash) служебные и в exe не попадают.
fn write_ui_table() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("web").join("ui");
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    files.sort();
    let mut table = String::from("&[
");
    for file in &files {
        let relative = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let kind = match file.extension().and_then(|e| e.to_str()) {
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("woff2") => "font/woff2",
            Some("webp") => "image/webp",
            Some("png") => "image/png",
            Some("json") => "application/json",
            _ => "application/octet-stream",
        };
        table.push_str(&format!("    ({:?}, {:?}, include_bytes!({:?}) as &[u8]),
", format!("/ui/{relative}"), kind, file.display().to_string()));
    }
    table.push_str("]
");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ui_files.rs");
    std::fs::write(out, table).expect("запись таблицы web/ui");
}

/// Картинки окна «Что нового» (docs/images/whats-new/*) встраиваются в exe и отдаются
/// только по имени файла из этой таблицы: путь из адреса на диск не доходит.
fn write_whats_new_images() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("docs").join("images").join("whats-new");
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    files.sort();
    let mut table = String::from("&[
");
    for file in &files {
        let kind = match file.extension().and_then(|e| e.to_str()) {
            Some("webp") => "image/webp",
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            _ => continue,
        };
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        table.push_str(&format!("    ({:?}, {:?}, include_bytes!({:?}) as &[u8]),
", name, kind, file.display().to_string()));
    }
    table.push_str("]
");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("whats_new_images.rs");
    std::fs::write(out, table).expect("запись таблицы картинок");
}

/// Каталог навыков, агентов и приложений: каждый JSON из catalog/** встраивается как текст.
/// Путь в таблице относительный (`skills/check-a-claim.json`), `index.json` лежит в корне.
fn write_catalog_table() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // Публичный каталог и, если папка есть, private/catalog (официальная сборка).
    // Индекс приватного каталога встраивается как `index.private.json`.
    let mut table = String::from("&[\n");
    for (root, private) in [(manifest.join("catalog"), false), (manifest.join("private").join("catalog"), true)] {
        let mut files = Vec::new();
        collect_files(&root, &mut files);
        files.retain(|f| f.extension().and_then(|e| e.to_str()) == Some("json"));
        files.sort();
        for file in &files {
            let mut relative = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            if private && relative == "index.json" {
                relative = "index.private.json".to_string();
            }
            table.push_str(&format!("    ({:?}, include_str!({:?})),\n", relative, file.display().to_string()));
        }
    }
    table.push_str("]\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("catalog_files.rs");
    std::fs::write(out, table).expect("запись таблицы каталога");
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else if !entry.file_name().to_string_lossy().starts_with('.') {
            files.push(path);
        }
    }
}

fn rust_lld() -> PathBuf {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let output = Command::new(rustc).args(["--print", "sysroot"]).output().expect("rustc --print sysroot");
    let sysroot = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let host = std::env::var("HOST").unwrap();
    Path::new(&sysroot).join("lib").join("rustlib").join(host).join("bin").join("rust-lld.exe")
}

fn build_import_libs(defs: &Path, out_dir: &Path) {
    let lld = rust_lld();
    for entry in std::fs::read_dir(defs).expect("папка link") {
        let def = entry.unwrap().path();
        if def.extension().and_then(|e| e.to_str()) != Some("def") {
            continue;
        }
        let name = def.file_stem().unwrap().to_string_lossy().into_owned();
        let target = out_dir.join(format!("lib{name}.a"));
        let status = Command::new(&lld)
            .args(["-flavor", "link", "/lib", "/machine:x64"])
            .arg(format!("/def:{}", def.display()))
            .arg(format!("/out:{}", target.display()))
            .status()
            .expect("запуск rust-lld");
        assert!(status.success(), "rust-lld не собрал импорт-библиотеку {name}");
    }
    println!("cargo:rustc-link-search=native={}", out_dir.display());
}

/// Иконка exe без rc/windres/cvtres: собираем COFF-объект (AMD64) с секцией `.rsrc`,
/// где лежит дерево ресурсов RT_ICON + RT_GROUP_ICON (id 1), и отдаём его линкеру.
/// RVA в записях данных — релокации ADDR32NB на символ секции.
fn build_icon_obj(manifest: &Path, out_dir: &Path) {
    // Официальная сборка берёт значок из private/assets (рисунок владельца), публичная — общий.
    let private_ico = manifest.join("private").join("assets").join("deloist.ico");
    if private_ico.is_file() {
        println!("cargo:rerun-if-changed={}", private_ico.display());
    }
    let ico_path = if private_ico.is_file() { private_ico } else { manifest.join("assets").join("deloist.ico") };
    println!("cargo:rerun-if-changed={}", ico_path.display());
    let ico = std::fs::read(&ico_path).expect("deloist.ico");
    assert!(ico.len() >= 6, "ico header");
    let n = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    assert!(n > 0, "ico has no images");

    // (ширина, высота, данные изображения)
    let mut images: Vec<(u8, u8, Vec<u8>)> = Vec::new();
    for i in 0..n {
        let p = 6 + i * 16;
        let size = u32::from_le_bytes(ico[p + 8..p + 12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(ico[p + 12..p + 16].try_into().unwrap()) as usize;
        assert!(off + size <= ico.len(), "ico image range");
        images.push((ico[p], ico[p + 1], ico[off..off + size].to_vec()));
    }
    let mut grp = Vec::new();
    grp.extend_from_slice(&[0, 0, 1, 0]);
    grp.extend_from_slice(&(n as u16).to_le_bytes());
    for (i, (w, h, d)) in images.iter().enumerate() {
        grp.extend_from_slice(&[*w, *h, 0, 0]);
        grp.extend_from_slice(&1u16.to_le_bytes());
        grp.extend_from_slice(&32u16.to_le_bytes());
        grp.extend_from_slice(&(d.len() as u32).to_le_bytes());
        grp.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }

    // Раскладка секции: корень, каталог иконок, каталог групп, по каталогу языка на ресурс,
    // записи данных, затем сами данные.
    let dir = |entries: usize| 16 + entries * 8;
    let root_off = 0usize;
    let icons_off = root_off + dir(2);
    let groups_off = icons_off + dir(n);
    let langs_off = groups_off + dir(1);
    let lang_size = dir(1);
    let total_res = n + 1;
    let entries_off = langs_off + total_res * lang_size;
    let data_start = entries_off + total_res * 16;
    let mut sec = vec![0u8; data_start];
    let mut relocs: Vec<u32> = Vec::new();
    let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put16 = |b: &mut Vec<u8>, at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    // Заголовок каталога: счётчик именованных = 0, счётчик по id = entries.
    let header = |b: &mut Vec<u8>, at: usize, entries: usize| put16(b, at + 14, entries as u16);
    header(&mut sec, root_off, 2);
    put32(&mut sec, root_off + 16, 3); // RT_ICON
    put32(&mut sec, root_off + 20, 0x8000_0000 | icons_off as u32);
    put32(&mut sec, root_off + 24, 14); // RT_GROUP_ICON
    put32(&mut sec, root_off + 28, 0x8000_0000 | groups_off as u32);
    header(&mut sec, icons_off, n);
    header(&mut sec, groups_off, 1);

    let blobs: Vec<&[u8]> = images.iter().map(|i| i.2.as_slice()).chain(std::iter::once(grp.as_slice())).collect();
    let mut data_pos = data_start;
    for (k, blob) in blobs.iter().enumerate() {
        let parent = if k < n { icons_off } else { groups_off };
        let id = if k < n { (k + 1) as u32 } else { 1 };
        let lang = langs_off + k * lang_size;
        let entry = entries_off + k * 16;
        let e = parent + 16 + if k < n { k } else { 0 } * 8;
        put32(&mut sec, e, id);
        put32(&mut sec, e + 4, 0x8000_0000 | lang as u32);
        header(&mut sec, lang, 1);
        put32(&mut sec, lang + 16, 0x0409);
        put32(&mut sec, lang + 20, entry as u32);
        // Запись данных: RVA (релокация), размер, кодовая страница, 0.
        data_pos = (data_pos + 3) & !3;
        put32(&mut sec, entry, data_pos as u32); // добавка к ADDR32NB
        put32(&mut sec, entry + 4, blob.len() as u32);
        relocs.push(entry as u32);
        sec.resize(data_pos, 0);
        sec.extend_from_slice(blob);
        data_pos += blob.len();
    }
    while sec.len() % 4 != 0 {
        sec.push(0);
    }

    // COFF: заголовок 20, секция 40, данные, релокации, один символ секции, таблица строк.
    let raw_ptr = 20 + 40;
    let reloc_ptr = raw_ptr + sec.len();
    let sym_ptr = reloc_ptr + relocs.len() * 10;
    let mut obj = Vec::new();
    obj.extend_from_slice(&0x8664u16.to_le_bytes()); // AMD64
    obj.extend_from_slice(&1u16.to_le_bytes());
    obj.extend_from_slice(&0u32.to_le_bytes());
    obj.extend_from_slice(&(sym_ptr as u32).to_le_bytes());
    obj.extend_from_slice(&1u32.to_le_bytes());
    obj.extend_from_slice(&0u16.to_le_bytes());
    obj.extend_from_slice(&0u16.to_le_bytes());
    obj.extend_from_slice(b".rsrc\0\0\0");
    obj.extend_from_slice(&0u32.to_le_bytes()); // VirtualSize
    obj.extend_from_slice(&0u32.to_le_bytes()); // VirtualAddress
    obj.extend_from_slice(&(sec.len() as u32).to_le_bytes());
    obj.extend_from_slice(&(raw_ptr as u32).to_le_bytes());
    obj.extend_from_slice(&(reloc_ptr as u32).to_le_bytes());
    obj.extend_from_slice(&0u32.to_le_bytes());
    obj.extend_from_slice(&(relocs.len() as u16).to_le_bytes());
    obj.extend_from_slice(&0u16.to_le_bytes());
    obj.extend_from_slice(&0x4030_0040u32.to_le_bytes()); // DATA | ALIGN_4 | READ
    obj.extend_from_slice(&sec);
    for r in &relocs {
        obj.extend_from_slice(&r.to_le_bytes());
        obj.extend_from_slice(&0u32.to_le_bytes()); // символ 0
        obj.extend_from_slice(&3u16.to_le_bytes()); // IMAGE_REL_AMD64_ADDR32NB
    }
    obj.extend_from_slice(b".rsrc\0\0\0");
    obj.extend_from_slice(&0u32.to_le_bytes()); // Value
    obj.extend_from_slice(&1u16.to_le_bytes()); // секция 1
    obj.extend_from_slice(&0u16.to_le_bytes());
    obj.push(3); // static
    obj.push(0);
    obj.extend_from_slice(&4u32.to_le_bytes()); // таблица строк пуста

    let obj_path = out_dir.join("deloist_icon.o");
    std::fs::write(&obj_path, &obj).expect("write deloist_icon.o");
    println!("cargo:rustc-link-arg-bins={}", obj_path.display());
}
