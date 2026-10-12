//! Embeds the icon and the version information in the Windows executable, so the program has the
//! same icon and a readable name in Task Manager and in the file properties.

fn main() {
    println!("cargo:rerun-if-changed=../packaging/windows/crabytube.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../packaging/windows/crabytube.ico")
            .set("ProductName", "CrabyTube")
            .set("FileDescription", "CrabyTube BotGuard helper")
            .set("InternalName", "botguard-helper")
            .set("OriginalFilename", "botguard-helper.exe")
            .set("LegalCopyright", "Licensed under GPL-3.0-or-later");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed the Windows icon: {error}");
        }
    }
}
