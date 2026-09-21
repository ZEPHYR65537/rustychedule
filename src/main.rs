fn main() {
    if let Err(e) = schedule::cli::run() {
        if e.to_string() != "__reported__" {
            eprintln!("错误：{e:#}");
        }
        std::process::exit(2);
    }
}
