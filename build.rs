fn main() {
    slint_build::compile("src/app.slint").expect("failed to compile the Slint UI");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_manifest_file("assets/app.manifest")
            .compile()
            .expect("failed to compile Windows resources");
    }
}
