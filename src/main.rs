fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = srun::cli::run(args);
    std::process::exit(code);
}
