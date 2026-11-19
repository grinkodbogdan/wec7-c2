fn main() {
    // Link at the component image base (see libs/rt/user.ld).
    println!("cargo:rustc-link-arg-bins=-Tuser.ld");
}
