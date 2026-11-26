// Embeds the app icon and file description into the .exe so Explorer shows them.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("FileDescription", "Slim Steam Switch");
        res.set("ProductName", "Slim Steam Switch");
        if let Err(e) = res.compile() {
            // A release (built on CI) must have its icon; a local build can go without.
            if std::env::var_os("CI").is_some() {
                panic!("exe icon not embedded: {e}");
            }
            println!("cargo:warning=exe icon not embedded: {e}");
        }
    }
}
