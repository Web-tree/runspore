//! Embeds the catalogue's example folders, so `spore explain` works offline.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("catalogue");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    for sub in ["examples", "fixed"] {
        walk(&root.join(sub), &mut files);
    }
    files.sort();
    let mut code = String::from("/// Every example file, by its path under `catalogue/`.\n");
    code.push_str("pub static FILES: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        code.push_str(&format!(
            "    ({rel:?}, include_bytes!({:?})),\n",
            file.display().to_string()
        ));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("catalogue_files.rs");
    std::fs::write(out, code).unwrap();
}
