fn main() {
    if let Err(e) = tongchou::cli::run() {
        if e.to_string() != "__reported__" {
            eprintln!("错误：{e:#}");
        }
        std::process::exit(2);
    }
}
