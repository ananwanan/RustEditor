fn main() {
    println!(
        "cargo:rustc-env=RUSTEDITOR_TARGET={}",
        std::env::var("TARGET").expect("Cargo target")
    );
}
