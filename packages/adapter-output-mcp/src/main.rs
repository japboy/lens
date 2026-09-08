#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = adapter_output_mcp::run_stdio().await {
        eprintln!("lens-output-mcp: {error}");
        std::process::exit(1);
    }
}
