use {
    tokio::sync::broadcast,
    tokio_stream::{wrappers::BroadcastStream, StreamExt},
    tonic::{transport::Server, Request, Response, Status},
    ordered_float::OrderedFloat,
    std::collections::BTreeMap,
    crate::types::Exchange,
};

// generated code from proto
pub mod orderbook {
    tonic::include_proto!("orderbook");
}

use orderbook::{
    orderbook_aggregator_server::{OrderbookAggregator, OrderbookAggregatorServer},
    Empty, Summary, Level,
};

pub struct OrderbookService {
    summary_tx: broadcast::Sender<Summary>,
}

impl OrderbookService {
    pub fn new(summary_tx: broadcast::Sender<Summary>) -> Self {
        Self { summary_tx }
    }

    pub fn into_server(self) -> OrderbookAggregatorServer<Self> {
        OrderbookAggregatorServer::new(self)
    }
}

#[tonic::async_trait]
impl OrderbookAggregator for OrderbookService {
    type BookSummaryStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<Summary, Status>> + Send + 'static>
    >;

    async fn book_summary(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<Self::BookSummaryStream>, Status> {
        tracing::info!("New gRPC client connected");

        let rx = self.summary_tx.subscribe();
        let stream = BroadcastStream::new(rx)
            .filter_map(|result| match result {
                Ok(summary) => Some(Ok(summary)),
                Err(e) => {
                    tracing::warn!("Broadcast error: {}", e);
                    None
                }
            });

        Ok(Response::new(Box::pin(stream)))
    }
}

pub fn create_summary(
    spread: f64,
    bids: &BTreeMap<(OrderedFloat<f64>, Exchange), f64>,
    asks: &BTreeMap<(OrderedFloat<f64>, Exchange), f64>,
    max_levels: usize,
) -> Summary {
    let bid_count = bids.len().min(max_levels);
    let ask_count = asks.len().min(max_levels);

    let mut bid_levels = Vec::with_capacity(bid_count);
    let mut ask_levels = Vec::with_capacity(ask_count);

    for ((price, exchange), amount) in bids.iter().rev().take(max_levels) {
        bid_levels.push(Level {
            exchange: exchange.to_string(),
            price: price.0,
            amount: *amount,
        });
    }

    for ((price, exchange), amount) in asks.iter().take(max_levels) {
        ask_levels.push(Level {
            exchange: exchange.to_string(),
            price: price.0,
            amount: *amount,
        });
    }

    Summary {
        spread,
        bids: bid_levels,
        asks: ask_levels,
    }
}

pub async fn run_grpc_server(
    addr: std::net::SocketAddr,
    summary_tx: broadcast::Sender<Summary>,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = OrderbookService::new(summary_tx);

    tracing::info!("Starting gRPC server on {}", addr);

    Server::builder()
        .add_service(service.into_server())
        .serve(addr)
        .await?;

    Ok(())
}
