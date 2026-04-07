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

    let square_svg = Path::new("assets/icons/icon-square.svg");
    let tray_svg = Path::new("assets/icons/icon.svg");
    let symbolic_svg = Path::new("assets/icons/icon-symbolic.svg");

    println!("cargo:rerun-if-changed=assets/icons/icon-square.svg");
    println!("cargo:rerun-if-changed=assets/icons/icon.svg");
    println!("cargo:rerun-if-changed=assets/icons/icon-symbolic.svg");

    // Window icon (256x256 raw RGBA for iced window::icon::from_rgba)
    let pixmap = render_svg(tray_svg, 256);
    save_raw_rgba(&pixmap, &out.join("icon-window-256.rgba"));

    // macOS .icns sizes (individual PNGs — iconutil runs in CI)
    for &size in &[16, 32, 64, 128, 256, 512, 1024] {
        let pixmap = render_svg(square_svg, size);
        save_png(&pixmap, &out.join(format!("icon-square-{size}.png")));
    }

    // Tray icon (32x32 raw RGBA) — colorful, used on Linux
    let pixmap = render_svg(tray_svg, 32);
    save_raw_rgba(&pixmap, &out.join("icon-tray-32.rgba"));

    // Symbolic tray icon (32x32 raw RGBA) — monochrome, used on macOS (template)
    // where it adapts to the system theme.
    let pixmap = render_svg(symbolic_svg, 32);
    save_raw_rgba(&pixmap, &out.join("icon-tray-symbolic-32.rgba"));

    // Windows executable icon (.ico embedded via resource file)
    #[cfg(target_os = "windows")]
    {
        let ico_path = out.join("cocuyo.ico");
        let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
        for &size in &[16u32, 32, 48, 64, 128, 256] {
            let pixmap = render_svg(square_svg, size);
            // tiny-skia is premultiplied; unpremultiply for ICO
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
            let image = ico::IconImage::from_rgba_data(size, size, rgba);
            icon_dir.add_entry(
                ico::IconDirEntry::encode(&image).expect("Failed to encode ICO entry"),
            );
        }
        let file = std::fs::File::create(&ico_path).expect("Failed to create .ico file");
        icon_dir.write(file).expect("Failed to write .ico file");

        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("ico path is not valid UTF-8"));
        res.compile().expect("Failed to compile Windows resource");
    }
}
