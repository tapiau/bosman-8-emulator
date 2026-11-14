//! Windows: embed the icon (ship's wheel with an "8", packaging/icons) into
//! the executables. Nothing to do on other systems.

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/icons/bosman8.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icons/bosman8.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
