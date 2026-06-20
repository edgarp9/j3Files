fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=icon.ico");
    println!("cargo:rerun-if-changed=assets/icons/nav_back.ico");
    println!("cargo:rerun-if-changed=assets/icons/nav_forward.ico");
    println!("cargo:rerun-if-changed=assets/icons/nav_up.ico");
    println!("cargo:rerun-if-changed=assets/icons/nav_refresh.ico");
    println!("cargo:rerun-if-changed=assets/icons/nav_go.ico");
    println!("cargo:rerun-if-changed=assets/icons/search_cancel.ico");

    let resource_result =
        embed_resource::compile("app.rc", embed_resource::NONE).manifest_required();
    if let Err(error) = resource_result {
        eprintln!("failed to compile Windows resources from app.rc: {error}");
        std::process::exit(1);
    }
}
