fn main() {
    if let Err(error) = breeze_runner::main_entry(std::env::args().collect()) {
        eprintln!("breeze-runner: {error}");
        std::process::exit(1);
    }
}
