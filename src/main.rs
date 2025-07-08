mod consul;

use consul::NodeEvent;

use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let consul = consul::Consul::new("localhost", 8500);
    let mut stream = consul.watch_nodes("");

    while let Some(event) = stream.next().await {
        match event {
            NodeEvent::Added(node) => {
                println!("Node added: {:?}", node);
            }
            NodeEvent::Removed(node) => {
                println!("Node removed: {:?}", node);
            }
            NodeEvent::Updated(node) => {
                println!("Node updated: {:?}", node);
            }
            NodeEvent::Error(err) => {
                eprintln!("Error watching nodes: {}", err);
            }
        }
    }

    println!("Consul Reaper starting...");

    Ok(())
}
