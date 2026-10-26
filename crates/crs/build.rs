fn main() {
    println!("cargo::rustc-check-cfg=cfg(remote)");
    println!("cargo::rustc-check-cfg=cfg(threads)");
}
