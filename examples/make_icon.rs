// Генерирует assets/deloist.ico из той же пиксельной графики, что и значок окна/трея.
// Запуск: cargo run --example make_icon
#[path = "../src/icon_art.rs"]
mod icon_art;

use std::fs;
use std::path::PathBuf;

fn png_bytes(rgba: &[u8], size: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, size, size);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(rgba).expect("png data");
    }
    buf
}

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let sizes: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
    for &size in &sizes {
        let rgba = icon_art::icon_rgba(size);
        entries.push((size, png_bytes(&rgba, size)));
    }

    // ICONDIR
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0u8, 0]); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // type = icon
    ico.extend_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut offset = 6u32 + 16u32 * entries.len() as u32;
    for (size, png) in &entries {
        let w = if *size == 256 { 0u8 } else { *size as u8 };
        let h = w;
        ico.push(w);
        ico.push(h);
        ico.push(0); // color count
        ico.push(0); // reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bit count
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &entries {
        ico.extend_from_slice(png);
    }

    let out = manifest.join("assets").join("deloist.ico");
    fs::write(&out, &ico).expect("write ico");
    println!("wrote {} ({} bytes)", out.display(), ico.len());
}
