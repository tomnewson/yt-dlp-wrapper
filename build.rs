fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".into());
    slint_build::compile_with_config("src/app.slint", config)
        .expect("failed to compile the Slint UI");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_manifest_file("assets/app.manifest")
            .compile()
            .expect("failed to compile Windows resources");
    }
}
