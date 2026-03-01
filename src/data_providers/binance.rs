use {
    crate::{
        net_utils::ws as ws_utils,
        types::{OrderBook, Exchange},
    },
    anyhow::Context,
    serde::Deserialize,
    serde_with::{serde_as, DisplayFromStr},
    tokio::sync::watch,
    std::error::Error as StdError,
};

pub struct Binance<D> {
    incoming_events_tx: watch::Sender<D>,
    last_update_id: Option<u64>,
}

impl<D> Binance<D> {
    pub fn new(incoming_events_tx: watch::Sender<D>) -> Self {
        Self {
            incoming_events_tx,
            last_update_id: None,
        }
    }
}

impl<D> ws_utils::ClientBuilder for Binance<D>
where
    D: TryFrom<OrderBookUpdate> + Send + Sync + 'static,
    <D as TryFrom<OrderBookUpdate>>::Error: StdError + Send + Sync,
{
    type Client<'a> = &'a mut Self where D: 'a;

    async fn build<'a>(
        &'a mut self,
        _: ws_utils::WsSink,
        _: &mut ws_utils::WsStream,
    ) -> anyhow::Result<Self::Client<'a>> {
        // no subscription needed, as it is done via query params in connection url
        Ok(self)
    }
}

impl<D> ws_utils::Client for Binance<D>
where
    D: TryFrom<OrderBookUpdate> + Send + Sync + 'static,
    <D as TryFrom<OrderBookUpdate>>::Error: StdError + Send + Sync,
{
    async fn on_text_message(&mut self, message: String) -> anyhow::Result<()> {
        tracing::debug!("on_text_message {}", message);

        let event: OrderBookUpdate = serde_json::from_str(message.as_str())?;

        match self.last_update_id {
            Some(local_id) if event.last_update_id <= local_id => {
                tracing::debug!(
                    "ignoring old snapshot: local={}, received={}",
                    local_id,
                    event.last_update_id
                );
                return Ok(());
            }
            _ => self.send_event(event)?,
        }

        Ok(())
    }
}

impl<D> Binance<D>
where
    D: TryFrom<OrderBookUpdate> + Send + Sync + 'static,
    <D as TryFrom<OrderBookUpdate>>::Error: StdError + Send + Sync,
{
    fn send_event(&mut self, event: OrderBookUpdate) -> anyhow::Result<()> {
        let update_id = event.last_update_id;
        let data = D::try_from(event).context("D::try_from")?;
        self.incoming_events_tx.send(data)?;
        self.last_update_id = Some(update_id);
        Ok(())
    }
}

impl TryFrom<OrderBookUpdate> for OrderBook {
    type Error = Error;

    fn try_from(ob: OrderBookUpdate) -> Result<Self, Self::Error> {
        Ok(OrderBook {
            exchnage: Exchange::Binance,
            asks: ob.asks,
            bids: ob.bids,
            received_at: std::time::SystemTime::now(),
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
}

#[serde_as]
#[derive(Deserialize, Debug, Clone)]
pub struct OrderBookUpdate {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    #[serde_as(as = "Vec<[DisplayFromStr; 2]>")]
    pub bids: Vec<[f64; 2]>,
    #[serde_as(as = "Vec<[DisplayFromStr; 2]>")]
    pub asks: Vec<[f64; 2]>,
}
