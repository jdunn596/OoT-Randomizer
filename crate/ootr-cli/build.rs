#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, warnings)]
#![forbid(unsafe_code)]

fn main() {
    pyo3_build_config::add_python_framework_link_args();
}
