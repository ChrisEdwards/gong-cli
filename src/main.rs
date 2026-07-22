#[tokio::main]
async fn main() {
    let exit_code = match gong_cli::run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("[FAIL] {error:#}");
            1
        }
    };

    std::process::exit(exit_code);
}
