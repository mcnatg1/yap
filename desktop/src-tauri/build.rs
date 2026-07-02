fn main() {
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=icons/128x128@2x.png");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=../crispasr-version.txt");
    println!("cargo:rerun-if-changed=../scripts/install-crispasr.ps1");
    println!("cargo:rerun-if-changed=../scripts/install-model.ps1");
    println!("cargo:rerun-if-changed=../scripts/install-all.ps1");
    println!("cargo:rerun-if-changed=../scripts/run-hidden.vbs");
    tauri_build::build()
}
