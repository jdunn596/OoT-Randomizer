#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

use {
    std::{
        env,
        io,
    },
    winresource::WindowsResource,
};

fn main() -> io::Result<()> {
    pyo3_build_config::add_python_framework_link_args();
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        WindowsResource::new()
            .set_icon("../../assets/ootr-arrows.ico")
            .set_manifest_file("assets/manifest.xml")
            .compile()?;
    }
    Ok(())
}
