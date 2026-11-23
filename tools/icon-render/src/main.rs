//! Renders the app icon SVG into the PNG sizes of a macOS `.iconset`, plus a raw RGBA image that
//! the app embeds for its Dock icon when run outside a bundle.
//!
//! Usage: icon-render <icon.svg> <out.iconset> <out.rgba>

use std::path::Path;
use std::{env, fs, process};

use resvg::{tiny_skia, usvg};

/// Size of the raw RGBA icon embedded in the app (see `src/main.rs`).
const RUNTIME_ICON_SIZE: u32 = 256;

/// (file name, pixel size) pairs `iconutil` expects in an `.iconset`.
const ICONSET: &[(&str, u32)] = &[
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

fn render(tree: &usvg::Tree, size: u32) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("icon size must be non-zero");
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap
}

fn run(svg: &Path, iconset: &Path, rgba: &Path) -> Result<(), String> {
    let data = fs::read(svg).map_err(|e| format!("can't read {}: {e}", svg.display()))?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|e| format!("invalid SVG {}: {e}", svg.display()))?;

    fs::create_dir_all(iconset).map_err(|e| format!("can't create {}: {e}", iconset.display()))?;
    for (name, size) in ICONSET {
        let path = iconset.join(name);
        render(&tree, *size)
            .save_png(&path)
            .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    }

    // tiny-skia stores premultiplied alpha; egui's IconData wants it unmultiplied.
    let pixels: Vec<u8> = render(&tree, RUNTIME_ICON_SIZE)
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    fs::write(rgba, pixels).map_err(|e| format!("can't write {}: {e}", rgba.display()))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [svg, iconset, rgba] = args.as_slice() else {
        eprintln!("usage: icon-render <icon.svg> <out.iconset> <out.rgba>");
        process::exit(2);
    };
    if let Err(e) = run(Path::new(svg), Path::new(iconset), Path::new(rgba)) {
        eprintln!("icon-render: {e}");
        process::exit(1);
    }
}
