// Windows: Programmsymbol in die .exe einbetten (Explorer, Taskleiste)
fn main() {
    println!("cargo:rerun-if-changed=assets/rustwave.rc");
    println!("cargo:rerun-if-changed=assets/logo/rustwave.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/rustwave.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
