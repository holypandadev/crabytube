//! Embeds the application icon in Windows executables. Other platforms take
//! their icon from packaging files (see `packaging/`).

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/crabytube.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("packaging/windows/crabytube.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed the Windows icon: {error}");
        }
    }
}
