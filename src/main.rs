use anyhow::Result;
use iron::IronNode;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let node = IronNode::new().await?;
    node.start().await?;

    Ok(())
}
