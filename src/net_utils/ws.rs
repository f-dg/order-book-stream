use {
    super::{
        retry::Retryable,
        tls,
    },
    bytes::Bytes,
    futures_util::{
        Future,
        Stream,
        StreamExt,
        stream::{SplitSink, SplitStream},
    },
    http,
    tokio::{
        time,
        net::{TcpSocket, TcpStream},
    },
    tokio_tungstenite::{
        WebSocketStream,
        MaybeTlsStream,
        tungstenite::Message,
        client_async_tls_with_config,
        Connector,
    },
    tungstenite,
    std::{
        io,
        net::{self, ToSocketAddrs},
        str::FromStr,
        time::Duration,
    },
};

pub type WsFull = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub type WsMessage = Message;
pub type WsSink = SplitSink<WsFull, WsMessage>;
pub type WsStream = SplitStream<WsFull>;

pub trait ClientBuilder {
    type Client<'a>: Client where Self: 'a;
    fn build<'a>(&'a mut self, tx: WsSink, rx: &mut WsStream)
        -> impl Future<Output = anyhow::Result<Self::Client<'a>>> + Send;
}

pub trait Client {
    fn on_tick(&mut self, _now: tokio::time::Instant)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        futures_util::future::ready(Ok(()))
    }

    fn on_send(&mut self)
        -> impl Future<Output = anyhow::Result<WsMessage>> + Send
    {
        futures_util::future::pending()
    }

    fn send(&mut self, _msg: WsMessage)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        futures_util::future::ready(Ok(()))
    }

    fn on_binary_message(&mut self, _message: Vec<u8>)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        futures_util::future::ready(Err(anyhow::Error::msg("unexpected binary message")))
    }

    fn on_text_message(&mut self, _message: String)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        futures_util::future::ready(Err(anyhow::Error::msg("unexpected text message")))
    }
}

impl<'a, C: Client> Client for &'a mut C {
    fn on_tick(&mut self, now: tokio::time::Instant)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        (**self).on_tick(now)
    }

    fn on_send(&mut self)
        -> impl Future<Output = anyhow::Result<WsMessage>> + Send
    {
        (**self).on_send()
    }

    fn send(&mut self, msg: WsMessage)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        (**self).send(msg)
    }

    fn on_binary_message(&mut self, message: Vec<u8>)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        (**self).on_binary_message(message)
    }

    fn on_text_message(&mut self, message: String)
        -> impl Future<Output = anyhow::Result<()>> + Send
    {
        (**self).on_text_message(message)
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub retry_delay: Duration,
    pub tick_interval: Duration,
    pub uri: http::Uri,

    pub max_frame_size: Option<usize>,
    pub max_message_size: Option<usize>,
}

impl From<Config> for tungstenite::protocol::WebSocketConfig {
    fn from(cnf: Config) -> Self {
        let mut ws_cnf = Self::default();
        ws_cnf.max_message_size = cnf.max_message_size;
        ws_cnf.max_frame_size = cnf.max_frame_size;
        ws_cnf
    }
}

impl Config {
    pub fn new(uri: http::Uri) -> Self {
        Self {
            retry_delay: Duration::from_secs(1),
            tick_interval: Duration::from_secs(1),
            uri,
            max_frame_size: None,
            max_message_size: None,
        }
    }

    pub fn with_retry_delay(mut self, retry_delay: Duration) -> Self {
        self.retry_delay = retry_delay;
        self
    }

    pub fn with_tick_interval(mut self, tick_interval: Duration) -> Self {
        self.tick_interval = tick_interval;
        self
    }

    pub fn with_max_frame_size(mut self, max_frame_size: usize) -> Self {
        self.max_frame_size = Some(max_frame_size);
        self
    }

    pub fn with_max_message_size(mut self, max_message_size: usize) -> Self {
        self.max_message_size = Some(max_message_size);
        self
    }
}

#[derive(thiserror::Error, Debug)]
pub enum PollError {
    #[error(transparent)]
    Connect(io::Error),
    #[error(transparent)]
    WsConnect(tungstenite::Error),
    #[error("parsing address error: {0}")]
    AddressError(AddressError),
    #[error(transparent)]
    ClientError(anyhow::Error),
    #[error(transparent)]
    Recv(RecvError<tungstenite::Error>),
}

impl Retryable for PollError {
    fn should_retry(&self) -> bool {
        match self {
            Self::Connect(err) => err.should_retry(),
            Self::WsConnect(err) => err.should_retry(),
            Self::AddressError(_) => false,
            Self::ClientError(_) => false,
            Self::Recv(err) => err.should_retry(),
        }
    }
}

pub async fn run_ws_loop<C: ClientBuilder>(
    builder: &mut C,
    config: Config,
) -> Result<(), PollError> {
    loop {
        match poll(builder, &config).await {
            Ok(()) => return Ok(()),
            Err(err) if err.should_retry() => {
                tracing::warn!(?err, "connection failed, reconnecting");
                time::sleep(config.retry_delay).await;
            }
            Err(err) => {
                tracing::error!(?err, "connection failed, exiting");
                return Err(err);
            }
        }
    }
}

pub async fn poll<C: ClientBuilder>(
    builder: &mut C,
    config: &Config,
) -> Result<(), PollError> {
    let connect_address = address_from_uri(&config.uri)
        .map_err(PollError::AddressError)?;

    tracing::info!(address = %connect_address, "opening tcp stream");

    let socket = TcpSocket::new_v4().map_err(PollError::Connect)?;
    let stream = socket.connect(connect_address)
        .await
        .map_err(PollError::Connect)?;

    let tls::HttpsClientConfig {
        tls_config, ..
    } = tls::webpki_root_cert_store().into();

    tracing::info!(uri = %config.uri, "connecting ws uri");
    let (ws_stream, _) = client_async_tls_with_config(
        &config.uri,
        stream,
        Some(config.clone().into()),
        Some(Connector::Rustls(tls_config.into())),
    ).await.map_err(PollError::WsConnect)?;

    tracing::info!(uri = %config.uri, "connected");
    poll_connection(builder, config, ws_stream).await
}

pub async fn poll_connection<C: ClientBuilder>(
    builder: &mut C,
    config: &Config,
    ws_stream: WsFull,
) -> Result<(), PollError> {
    let (ws_tx, mut ws_rx) = ws_stream.split();

    let mut client = match builder.build(ws_tx, &mut ws_rx).await {
        Ok(res) => res,
        Err(err) => {
            tracing::error!(?err, "client build error");
            return Err(PollError::ClientError(err));
        }
    };
    let mut tick_interval = tokio::time::interval(config.tick_interval);

    loop {
        tokio::select! {
            now = tick_interval.tick() => {
                if let Err(err) = client.on_tick(now).await {
                    tracing::error!(?err, "client on_tick error");
                    return Err(PollError::ClientError(err));
                }
            }
            out_msg = client.on_send() => {
                if let Err(err) = client.send(out_msg.map_err(PollError::ClientError)?).await {
                    tracing::error!(?err, "client send message error");
                    return Err(PollError::ClientError(err));
                }
            }
            msg = next_message(&mut ws_rx) => {
                match msg.map_err(PollError::Recv)? {
                    WsMessage::Ping(_) => {},
                    WsMessage::Pong(_) => {},
                    WsMessage::Binary(msg) => {
                        if let Err(err) = client.on_binary_message(msg.to_vec()).await {
                            tracing::error!(?err, "client on_binary_message error");
                            return Err(PollError::ClientError(err));
                        }
                    }
                    WsMessage::Text(msg) => {
                        if let Err(err) = client.on_text_message(msg.to_string()).await {
                            tracing::error!(?err, "client on_text_message error");
                            return Err(PollError::ClientError(err));
                        }
                    }
                    WsMessage::Close(_) =>
                        return Err(PollError::Recv(RecvError::Close)),
                    WsMessage::Frame(_) =>
                        return Err(PollError::Recv(RecvError::UnexpectedFrame)),
                }
            }
        }
    }
}

pub async fn parse_next_message<T, S, E>(
    ws_rx: &mut S,
) -> Result<T, RecvError<E>>
where
    T: FromStr,
    S: Stream<Item = Result<WsMessage, E>> + Unpin,
{
    loop {
        if let Some(msg) = next_message(ws_rx).await.and_then(parse_message)? {
            break Ok(msg);
        }
    }
}

pub async fn next_binary_message<S, E>(
    ws_rx: &mut S,
) -> Result<Bytes, RecvError<E>>
where
    S: Stream<Item = Result<WsMessage, E>> + Unpin,
{
    loop {
        if let Some(msg) = next_message(ws_rx).await.and_then(to_binary_message)? {
            break Ok(msg);
        }
    }
}

async fn next_message<T, S, E>(
    ws_rx: &mut S,
) -> Result<T, RecvError<E>>
where
    S: Stream<Item = Result<T, E>> + Unpin,
{
    match ws_rx.next().await {
        Some(Ok(msg)) => Ok(msg),
        Some(Err(err)) => Err(RecvError::Protocol(err)),
        None => Err(RecvError::Close),
    }
}

pub fn parse_message<T: FromStr, E>(
    message: WsMessage,
) -> Result<Option<T>, RecvError<E>> {
    match message {
        WsMessage::Ping(_) => Ok(None),
        WsMessage::Pong(_) => Ok(None),
        WsMessage::Close(_) => Err(RecvError::Close),
        WsMessage::Binary(_) => Err(RecvError::UnexpectedBinary),
        WsMessage::Text(msg) => {
            let res = msg.parse()
               .map_err(|_|  RecvError::InvalidTextMsg(
                   String::from_utf8_lossy(msg.as_bytes()).into())
               )?;
            Ok(Some(res))
        },
        WsMessage::Frame(_) => Err(RecvError::UnexpectedFrame),
    }
}

pub fn to_binary_message<E>(
    message: WsMessage,
) -> Result<Option<Bytes>, RecvError<E>> {
    match message {
        WsMessage::Ping(_) => Ok(None),
        WsMessage::Pong(_) => Ok(None),
        WsMessage::Close(_) => Err(RecvError::Close),
        WsMessage::Binary(msg) => Ok(Some(msg)),
        WsMessage::Text(_) => Err(RecvError::UnexpectedText),
        WsMessage::Frame(_) => Err(RecvError::UnexpectedFrame),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecvError<E> {
    #[error("received ws close")]
    Close,
    #[error("received binary on text connection")]
    UnexpectedBinary,
    #[error("received text on binary connection")]
    UnexpectedText,
    #[error("invalid text: {0}")]
    InvalidTextMsg(String),
    #[error("received ws frame")]
    UnexpectedFrame,
    #[error(transparent)]
    Protocol(E),
}

impl<E: Retryable> Retryable for RecvError<E> {
    fn should_retry(&self) -> bool {
        match self {
            Self::Close => true,
            Self::UnexpectedBinary => false,
            Self::UnexpectedText => false,
            Self::UnexpectedFrame => false,
            Self::InvalidTextMsg(_) => false,
            Self::Protocol(err) => err.should_retry(),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum AddressError<E = io::Error> {
    #[error("invalid host")]
    InvalidHost,
    #[error("invalid port")]
    InvalidPort,
    #[error("could not derive address from uri")]
    ParseError(#[from] E),
    #[error("empty socket address")]
    EmptySocketAddr,
}

pub fn address_from_uri<E>(uri: &http::Uri) -> Result<net::SocketAddr, AddressError<E>>
where
    AddressError<E>: From<io::Error>,
{
    let mut iter_address = (
        uri.host().ok_or(AddressError::InvalidHost)?,
        uri.port_u16().map_or_else(|| match uri.scheme_str() {
                Some("http") => Ok(80),
                Some("https") => Ok(443),
                Some("ws") => Ok(80),
                Some("wss") => Ok(443),
                _ => return Err(AddressError::InvalidPort),
            },
            Ok,
        )?,
    ).to_socket_addrs()?;

    Ok(iter_address.next().ok_or(AddressError::EmptySocketAddr)?)
}

pub fn text_message<T: Into<tungstenite::Utf8Bytes>>(msg: T) -> WsMessage {
    WsMessage::Text(msg.into())
}
