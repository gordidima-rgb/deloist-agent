//! Пиксельная графика значка Deloist: скруглённый квадрат цвета бренда с галочкой.
//! Один источник истины для значка окна/трея (src/desktop.rs) и для встраиваемого в exe .ico (examples/make_icon.rs).

/// Возвращает RGBA-пиксели значка размером size×size.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 9.0 / 32.0;
    // Галочка из знака Deloist: на маленьком значке контур и искры не читаются.
    let check = [(0.27 * s, 0.53 * s), (0.44 * s, 0.70 * s), (0.75 * s, 0.33 * s)];
    let to_segment = |x: f32, y: f32, a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
    };
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let dx = (radius - x).max(x - (s - radius)).max(0.0);
            let dy = (radius - y).max(y - (s - radius)).max(0.0);
            if dx.hypot(dy) > radius {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let distance = to_segment(x, y, check[0], check[1]).min(to_segment(x, y, check[1], check[2]));
            let white = (s * 0.055 - distance + 0.5).clamp(0.0, 1.0);
            let mix = |from: f32| (from + (255.0 - from) * white) as u8;
            rgba.extend_from_slice(&[mix(0x02 as f32), mix(0x5b as f32), mix(0xff as f32), 255]);
        }
    }
    rgba
}
