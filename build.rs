use std::path::Path;

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

fn render_svg(svg_path: &Path, size: u32) -> Pixmap {
    let svg_data = std::fs::read(svg_path).expect("Failed to read SVG");
    let tree = Tree::from_data(&svg_data, &Options::default()).expect("Failed to parse SVG");

    let mut pixmap = Pixmap::new(size, size).expect("Failed to create pixmap");

    let svg_size = tree.size();
    let scale = size as f32 / svg_size.width().max(svg_size.height());
    let transform = Transform::from_scale(scale, scale);

    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
}

fn save_png(pixmap: &Pixmap, path: &Path) {
    let png_data = pixmap.encode_png().expect("Failed to encode PNG");
    std::fs::write(path, png_data).expect("Failed to write PNG");
}

fn save_raw_rgba(pixmap: &Pixmap, path: &Path) {
    // tiny-skia stores premultiplied alpha — unpremultiply before saving raw RGBA
    let src = pixmap.data();
    let mut rgba = Vec::with_capacity(src.len());
    for pixel in src.chunks_exact(4) {
        let (r, g, b, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);
        if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let a_f = a as f32 / 255.0;
            rgba.push((r as f32 / a_f).round().min(255.0) as u8);
            rgba.push((g as f32 / a_f).round().min(255.0) as u8);
            rgba.push((b as f32 / a_f).round().min(255.0) as u8);
            rgba.push(a);
        }
    }
    std::fs::write(path, rgba).expect("Failed to write raw RGBA");
}

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out = Path::new(&out_dir);

    let square_svg = Path::new("assets/icon-square.svg");
    let tray_svg = Path::new("assets/icon.svg");

    println!("cargo:rerun-if-changed=assets/icon-square.svg");
    println!("cargo:rerun-if-changed=assets/icon.svg");

    // Window icon (256x256 raw RGBA for iced window::icon::from_rgba)
    let pixmap = render_svg(tray_svg, 256);
    save_raw_rgba(&pixmap, &out.join("icon-window-256.rgba"));

    // macOS .icns sizes (individual PNGs — iconutil runs in CI)
    for &size in &[16, 32, 64, 128, 256, 512, 1024] {
        let pixmap = render_svg(square_svg, size);
        save_png(&pixmap, &out.join(format!("icon-square-{size}.png")));
    }

    // Tray icon (32x32 raw RGBA)
    let pixmap = render_svg(tray_svg, 32);
    save_raw_rgba(&pixmap, &out.join("icon-tray-32.rgba"));
}
