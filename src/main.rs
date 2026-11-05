//! The `monet` binary: the editor and the command line natively, the editor in
//! the browser (built for wasm32 by web/build.sh).

#[cfg(not(target_arch = "wasm32"))]
mod cli;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    cli::main()
}

#[cfg(target_arch = "wasm32")]
fn main() {
    monet::web::start();
}
