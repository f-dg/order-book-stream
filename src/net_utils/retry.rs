use {
    tungstenite,
    rand::random,
    tokio::time,
    std::{
        cmp,
        future,
        io,
        time::Duration,
    },
};

pub trait Retryable {
    fn should_retry(&self) -> bool;
}

impl Retryable for io::ErrorKind {
    fn should_retry(&self) -> bool {
        match self {
            Self::ConnectionRefused => true,
            Self::ConnectionReset => true,
            Self::HostUnreachable => true,
            Self::NetworkUnreachable => true,
            Self::ConnectionAborted => true,
            Self::NotConnected => true,
            Self::AddrInUse => true,
            Self::NetworkDown => true,
            Self::BrokenPipe => true,
            Self::WouldBlock => true,
            Self::TimedOut => true,
            Self::Interrupted => true,
            Self::UnexpectedEof => true,
            _ => false,
        }
    }
}

impl Retryable for io::Error {
    fn should_retry(&self) -> bool {
        self.kind().should_retry()
    }
}

impl Retryable for tungstenite::Error {
    fn should_retry(&self) -> bool {
        use tungstenite::error::ProtocolError;
        match self {
            Self::ConnectionClosed => true,
            Self::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
            Self::Protocol(ProtocolError::SendAfterClosing) => true,
            Self::Protocol(ProtocolError::ReceivedAfterClosing) => true,
            Self::Io(err) if err.should_retry() => true,
            _ => false,
        }
    }
}

pub struct Retry<F> {
    pub inner: F,
    pub backoff: ExponentialBackoff,
    pub max_retries: usize,
}

impl<F, Fut, T, E> Retry<F>
where
    F: Fn() -> Fut,
    Fut: future::Future<Output = Result<T, E>>,
    E: Retryable,
{
    pub const DEFAULT_MAX_RETRIES: usize = 5;
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            backoff: Default::default(),
            max_retries: Self::DEFAULT_MAX_RETRIES,
        }
    }

    pub async fn wrap(inner: F) -> Result<T, E> {
        Self::new(inner).run().await
    }

    pub async fn run(&mut self) -> Result<T, E> {
        let mut retries = 0;
        loop {
            match (self.inner)().await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if !e.should_retry() || retries >= self.max_retries {
                        return Err(e);
                    }
                }
            }

            let backoff = self.backoff.next();
            time::sleep(backoff).await;
            retries += 1;
        }
    }
}

#[derive(Debug)]
pub struct ExponentialBackoff {
    pub current_interval: Option<Duration>,
    pub initial_interval: Duration,
    pub max_interval: Duration,
    // multiplier.
    // next_interval = current_interval * factor * (1 + sample(-jitter, +jitter))
    pub jitter: f64,
    pub factor: f64,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            current_interval: None,
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(30),
            jitter: 0.5,
            factor: 2.0,
        }
    }
}

impl ExponentialBackoff {
    pub fn new_with_max(max_interval: Duration) -> Self {
        Self {
            max_interval,
            ..Default::default()
        }
    }

    pub fn reset(&mut self) {
        self.current_interval = None;
    }

    pub fn current_interval(&self) -> Duration {
        self.current_interval.unwrap_or(self.initial_interval)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Duration {
        let flat = match self.current_interval {
            Some(current) => current.as_secs_f64() * self.factor,
            None => self.initial_interval.as_secs_f64(),
        };
        self.current_interval = Some(cmp::min(Duration::from_secs_f64(flat), self.max_interval));

        let jitter = self.jitter * 2.0 * (random::<f64>() - 0.5);
        let jittered = Duration::from_secs_f64((flat * (1.0 + jitter)).max(0.0));
        cmp::min(jittered, self.max_interval)
    }
}
