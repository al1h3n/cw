//! Embeds the Windows taskbar/Explorer icon into the viewer executable (the "Host" icon).

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico");
        if let Err(err) = res.compile() {
            // A missing resource compiler shouldn't fail the whole build; the icon is cosmetic.
            println!("cargo:warning=could not embed the viewer icon: {err}");
        }
    }
}
