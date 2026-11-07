use std::path::Path;

const ICONS: [(u32, &str); 3] = [
    (1, "assets/sonear.ico"),
    (2, "assets/tray-white.ico"),
    (3, "assets/tray-black.ico"),
];

fn resource_script(manifest_dir: &Path) -> String {
    let number = |name: &str| std::env::var(name).unwrap_or_else(|_| "0".to_owned());
    let (major, minor, patch) = (
        number("CARGO_PKG_VERSION_MAJOR"),
        number("CARGO_PKG_VERSION_MINOR"),
        number("CARGO_PKG_VERSION_PATCH"),
    );
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let dotted = format!("{major}.{minor}.{patch}.0");
    let commas = format!("{major},{minor},{patch},0");
    let mut script = String::new();

    // GPUI loads the window icon from resource 1; the tray uses 2 (white) and 3 (black).
    for (id, icon) in ICONS {
        let path = manifest_dir
            .join(icon)
            .display()
            .to_string()
            .replace('\\', "/");

        script.push_str(&format!("{id} ICON \"{path}\"\n"));
    }

    script.push_str(&format!(
        "
1 VERSIONINFO
FILEVERSION {commas}
PRODUCTVERSION {commas}
FILEOS 0x00040004
FILETYPE 0x1
BEGIN
    BLOCK \"StringFileInfo\"
    BEGIN
        BLOCK \"040904B0\"
        BEGIN
            VALUE \"CompanyName\", \"Hypox Studios\"
            VALUE \"ProductName\", \"Sonear\"
            VALUE \"FileDescription\", \"Sonear\"
            VALUE \"ProductVersion\", \"{version}\"
            VALUE \"FileVersion\", \"{dotted}\"
            VALUE \"OriginalFilename\", \"sonear.exe\"
        END
    END
    BLOCK \"VarFileInfo\"
    BEGIN
        VALUE \"Translation\", 0x0409, 0x04B0
    END
END
"
    ));

    script
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=locales");

    for (_, icon) in ICONS {
        println!("cargo:rerun-if-changed={icon}");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let out_dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    let script = Path::new(&out_dir).join("sonear.rc");

    std::fs::write(&script, resource_script(Path::new(&manifest_dir)))
        .expect("could not write the Windows resource script");
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_optional()
        .expect("could not embed the Windows icon and version");
}
