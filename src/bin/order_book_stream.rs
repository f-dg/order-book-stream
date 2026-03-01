use {
    order_book_stream::{
        data_providers::{
            bitstamp::Bitstamp,
            binance::Binance,
        },
        order_book_stream::Runner,
        net_utils::ws,
        types::OrderBook,
        grpc_server::run_grpc_server,
    },
    http::Uri,
    tokio::{
        sync::{watch, broadcast},
        task,
    },
    rustls::crypto::{
        CryptoProvider,
        aws_lc_rs,
    },
    tokio_util::sync::CancellationToken,
    tracing_subscriber::{
        EnvFilter,
        layer::{Layer, SubscriberExt},
        util::SubscriberInitExt,
    },
};

#[tokio::main]
async fn main() {
    let ticker = get_ticker_from_cli_args()
        .expect("ticker required, provide via CLI arg: --ticker {ticker}")
        .to_lowercase();

    let _ = CryptoProvider::install_default(aws_lc_rs::default_provider());

    let filter = EnvFilter::from_default_env();
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .init();

    let (tx1, rx1) = watch::channel(OrderBook::default());
    let mut binance = Binance::new(tx1);

    let ws_uri: Uri = format!("wss://stream.binance.com:9443/ws/{}@depth20@100ms", ticker)
        .parse().expect("valid ticker");

    let join_handle1 = task::spawn(async move {
        let config = ws::Config::new(ws_uri);
        let res = ws::run_ws_loop(&mut binance, config).await;
        tracing::error!("binance ws_loop is stopped: {res:?}");
    });

    let ws_uri: Uri = "wss://ws.bitstamp.net".parse().expect("valid bitstamp url");
    let (tx2, rx2) = watch::channel(OrderBook::default());
    let mut bitstamp = Bitstamp::new(format!("order_book_{ticker}"), tx2);

    let join_handle2 = task::spawn(async move {
        let config = ws::Config::new(ws_uri);
        let res = ws::run_ws_loop(&mut bitstamp, config).await;
        tracing::error!("bitstamp ws_loop is stopped: {res:?}");
    });

    // Create broadcast channel for gRPC Summary messages
    let (summary_tx, _summary_rx) = broadcast::channel(100);

    // Start gRPC server
    let grpc_addr = "127.0.0.1:50051".parse().expect("valid grpc address");
    let grpc_summary_tx = summary_tx.clone();
    let grpc_handle = task::spawn(async move {
        if let Err(e) = run_grpc_server(grpc_addr, grpc_summary_tx).await {
            tracing::error!("gRPC server error: {}", e);
        }
    });

    let mut runner = Runner::new(rx1, rx2, summary_tx);
    let cancel_token = CancellationToken::new();

    let result = runner.run(cancel_token.child_token()).await;

    tracing::info!("order book stream stopped: {result:?}");

    join_handle1.abort();
    join_handle2.abort();
    grpc_handle.abort();
    cancel_token.cancel();
}

fn get_ticker_from_cli_args() -> Option<String> {
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg == "--ticker" || arg == "-t" {
            return args.next();
        }
    }

    None
}
