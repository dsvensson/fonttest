#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    fonttest_web::run_native()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
