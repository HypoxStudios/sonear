fn main() {
    println!("cargo:rerun-if-changed=sonear.rc");
    println!("cargo:rerun-if-changed=assets/sonear.ico");
    println!("cargo:rerun-if-changed=assets/tray-white.ico");
    println!("cargo:rerun-if-changed=assets/tray-black.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // GPUI loads the window icon from resource 1; the tray uses 2 (white) and 3 (black).
    embed_resource::compile("sonear.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("could not embed the Windows icon");
}
