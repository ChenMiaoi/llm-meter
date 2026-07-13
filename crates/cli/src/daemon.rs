#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    llm_meter_daemon::run().await
}
