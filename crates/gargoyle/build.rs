//! Makes cargo rebuild the binary when the embedded frontend (`web/dist`) changes or first appears.
//!
//! rust-embed only tracks the files it embedded, so without this a build made before `make web`
//! would keep serving the placeholder page.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let web = Path::new(&manifest_dir).join("../../web");
    let dist = web.join("dist");
    if dist.is_dir() {
        println!("cargo:rerun-if-changed={}", dist.display());
    } else if web.is_dir() {
        // Watching web/ notices dist/ being created. Only done while dist/ is missing, because
        // web/ also contains node_modules and sources that should not trigger rebuilds.
        println!("cargo:rerun-if-changed={}", web.display());
    }
}
