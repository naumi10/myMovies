fn main() {
    // Only run this build script if we are compiling for Windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico"); // Path to your converted icon
        res.compile().unwrap();
    }
}
