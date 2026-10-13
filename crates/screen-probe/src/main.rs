// Legacy native.rs/guardian.rs remain excluded. No input hook or cursor mutation.
#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod controller;
mod engine;
mod geometry;
#[cfg(windows)]
mod presentation;
#[cfg(windows)]
mod supervisor;

fn main() {
    #[cfg(windows)]
    if let Err(error) = controller::run() {
        eprintln!("screen-probe: {error}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    eprintln!("This feasibility probe requires Windows 11 x64.");
}
