#![forbid(unsafe_code)]

mod supervisor;
mod worker;
mod worker_io;

fn main() {
    if let Err(error) = worker::run() {
        eprintln!("hat-local-inference-manager-worker: {error}");
        std::process::exit(1);
    }
}
