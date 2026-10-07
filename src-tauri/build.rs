fn main() {
    println!("cargo:rerun-if-env-changed=NYATERM_GITHUB_GIST_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=NYATERM_PACKAGE_MANAGER");

    if let Ok(client_id) = std::env::var("NYATERM_GITHUB_GIST_CLIENT_ID") {
        println!("cargo:rustc-env=NYATERM_GITHUB_GIST_CLIENT_ID={client_id}");
    }

    if let Ok(package_manager) = std::env::var("NYATERM_PACKAGE_MANAGER") {
        println!("cargo:rustc-env=NYATERM_PACKAGE_MANAGER={package_manager}");
    }

    tauri_build::build();
}
