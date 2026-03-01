use {
    order_book_stream::grpc_server::orderbook::{
        orderbook_aggregator_client::OrderbookAggregatorClient,
        Empty,
    },
    tokio_stream::StreamExt,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut client = OrderbookAggregatorClient::connect("http://127.0.0.1:50051").await?;

    println!("Connected to gRPC server, streaming order book summaries...\n");

    let mut stream = client.book_summary(Empty {}).await?.into_inner();

    while let Some(summary) = stream.next().await {
        match summary {
            Ok(summary) => {
                println!("=== Order Book Summary ===");
                println!("Spread: {:.6}", summary.spread);

                println!("\nTop 10 Asks:");
                for (i, ask) in summary.asks.iter().take(10).enumerate() {
                    println!("  {}. {:.6} @ {:.8} {}", i + 1, ask.price, ask.amount, ask.exchange);
                }

                println!("\nTop 10 Bids:");
                for (i, bid) in summary.bids.iter().take(10).enumerate() {
                    println!("  {}. {:.6} @ {:.8} {}", i + 1, bid.price, bid.amount, bid.exchange);
                }

                println!("\n");
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }

    Ok(())
}
