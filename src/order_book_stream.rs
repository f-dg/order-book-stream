use {
    crate::{
        types::{OrderBook, Exchange},
        grpc_server::{orderbook::Summary, create_summary},
    },
    tokio::{
        select,
        sync::watch,
    },
    tokio::sync::broadcast,
    tokio_util::sync::CancellationToken,
    ordered_float::OrderedFloat,
    std::{
        collections::BTreeMap,
        fmt::Debug,
        time::{SystemTime, Duration},
    },
};

const MAX_DEPTH: usize = 10usize;

pub struct Runner {
    combined_asks: BTreeMap<(OrderedFloat<f64>, Exchange), f64>, // Key(price, exchange) -> Value(qty)
    combined_bids: BTreeMap<(OrderedFloat<f64>, Exchange), f64>, // Key(price, exchange) -> Value(qty)

    rx1: watch::Receiver<OrderBook>, // Binance
    rx2: watch::Receiver<OrderBook>, // Bitstamp

    summary_tx: broadcast::Sender<Summary>, // gRPC broadcast

    // latency tracking
    update_count: u64,
}


impl Runner {
    pub fn new(
        rx1: watch::Receiver<OrderBook>,
        rx2: watch::Receiver<OrderBook>,
        summary_tx: broadcast::Sender<Summary>,
    ) -> Self {
        Self {
            combined_asks: BTreeMap::default(),
            combined_bids: BTreeMap::default(),
            rx1, rx2,
            summary_tx,
            update_count: 0,
        }
    }

    pub async fn run(&mut self, cancel_token: CancellationToken) -> Result<(), Error> {
        loop {
            select! {
                _ = cancel_token.cancelled() => {
                    return Ok(());
                },
                res = self.rx1.changed() => {
                    let _ = res?;
                    let update_start = SystemTime::now();
                    let update = self.rx1.borrow().clone();
                    self.apply_update(update);
                    self.broadcast(update_start);
                }
                res = self.rx2.changed() => {
                    let _ = res?;
                    let update_start = SystemTime::now();
                    let update = self.rx2.borrow().clone();
                    self.apply_update(update);
                    self.broadcast(update_start);
                }
            }
        }
    }

    // crossed book validation not implemented (ask lower than bids or vice versa)
    fn apply_update(&mut self, update: OrderBook) {
        let exchange = update.exchnage.clone();

        for [price, qty] in update.asks {
            let key = (OrderedFloat(price), exchange.clone());
            if qty > 0. {
                self.combined_asks.insert(key, qty);
            } else {
                self.combined_asks.remove(&key);
            }
        }

        while self.combined_asks.len() > MAX_DEPTH {
            self.combined_asks.pop_last();
        }

        for [price, qty] in update.bids {
            let key = (OrderedFloat(price), exchange.clone());
            if qty > 0. {
                self.combined_bids.insert(key, qty);
            } else {
                self.combined_bids.remove(&key);
            }
        }

        while self.combined_bids.len() > MAX_DEPTH {
            self.combined_bids.pop_first();
        }
    }

    fn broadcast(&mut self, update_start: SystemTime) {
        self.update_count += 1;
        let latency = update_start.elapsed().unwrap_or(Duration::from_secs(0));

        let best_bid = self.combined_bids.iter().next_back().map(|((p, _), _)| p.0);
        let best_ask = self.combined_asks.iter().next().map(|((p, _), _)| p.0);

        let spread = match (best_bid, best_ask) {
            (Some(bid), Some(ask)) => ask - bid,
            _ => 0.0,
        };

        let summary = create_summary(
            spread,
            &self.combined_bids,
            &self.combined_asks,
            MAX_DEPTH,
        );

        // send() returns Err if no receivers, which is fine
        let _ = self.summary_tx.send(summary);

        if self.update_count % 10 == 0 {
            tracing::info!(
                "summary #{}: spread={:.4}, bids={}, asks={}, latency={}µs",
                self.update_count,
                spread,
                self.combined_bids.len(),
                self.combined_asks.len(),
                latency.as_micros(),
            );
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Watch(#[from] watch::error::RecvError),
}
