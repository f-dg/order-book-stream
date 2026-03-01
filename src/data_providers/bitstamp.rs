use {
    crate::{
        net_utils::ws as ws_utils,
        types::{OrderBook, Exchange},
    },
    anyhow::{Context, bail},
    futures_util::SinkExt,
    serde::Deserialize,
    serde_json::json,
    serde_with::{serde_as, DisplayFromStr},
    tokio::sync::watch,
    std::{
        error::Error as StdError,
        fmt::Debug,
        str::FromStr,
    },
};

pub struct Bitstamp<D> {
    channel: String,
    incoming_events_tx: watch::Sender<D>,
    last_timestamp: Option<u64>,
}

impl<D> Bitstamp<D> {
    pub fn new(channel: String, incoming_events_tx: watch::Sender<D>) -> Self {
        Self { channel, incoming_events_tx, last_timestamp: None }
    }
}

impl<D> ws_utils::ClientBuilder for Bitstamp<D>
where
    D: TryFrom<OrderBookUpdate> + Send + Sync + 'static,
    <D as TryFrom<OrderBookUpdate>>::Error: StdError + Send + Sync,
{
    type Client<'a> = &'a mut Self where D: 'a;

    async fn build<'a>(
        &'a mut self,
        mut sink: ws_utils::WsSink,
        rx: &mut ws_utils::WsStream,
    ) -> anyhow::Result<Self::Client<'a>> {

        let channel = json!({
            "event": "bts:subscribe",
            "data": {
                "channel": self.channel,
            }
        });

        sink.send(channel.to_string().into()).await
            .context(format!("sink.send: subscription to {}", self.channel))?;

        #[derive(Deserialize)]
        struct EventResponse {
            pub event: String,
        }

        impl FromStr for EventResponse {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                serde_json::from_str(s)
                    .map_err(|err| format!("EventResponse deserialization: {err:?}"))
            }
        }

        let event: EventResponse = ws_utils::parse_next_message(rx).await
            .context("subscription response")?;

        if event.event != "bts:subscription_succeeded" {
            bail!("failed to subscribe to {}", self.channel);
        }

        Ok(self)
    }
}

impl<D> ws_utils::Client for Bitstamp<D>
where
    D: TryFrom<OrderBookUpdate> + Send + Sync + 'static,
    <D as TryFrom<OrderBookUpdate>>::Error: StdError + Send + Sync,
{
    async fn on_text_message(&mut self, message: String) -> anyhow::Result<()> {
        tracing::debug!("on_text_message {}", message);

        let event: Event = serde_json::from_str(message.as_str())?;
        let update_ts = event.data.microtimestamp;
        let last_ts  = self.last_timestamp;

        let send = || -> anyhow::Result<()> {
            let data = D::try_from(event.data).context("D::try_from")?;

            self.incoming_events_tx.send(data)?;
            self.last_timestamp = Some(update_ts);

            Ok(())
        };

        match last_ts {
            Some(last_ts) if last_ts < update_ts => send()?,
            None => send()?,
            _ => (), // otherwise event is out of order
        }


        Ok(())
    }
}

impl TryFrom<OrderBookUpdate> for OrderBook {
    type Error = Error;

    fn try_from(ob: OrderBookUpdate) -> Result<Self, Self::Error> {
        Ok(OrderBook {
            exchnage: Exchange::Bitstamp,
            asks: ob.asks,
            bids: ob.bids,
            received_at: std::time::SystemTime::now(),
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
}

#[derive(Deserialize, Debug)]
pub struct Event {
    data: OrderBookUpdate,
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct OrderBookUpdate {
    pub timestamp: String,
    #[serde_as(as = "DisplayFromStr")]
    pub microtimestamp: u64,
    #[serde_as(as = "Vec<[DisplayFromStr; 2]>")]
    pub bids: Vec<[f64; 2]>,
    #[serde_as(as = "Vec<[DisplayFromStr; 2]>")]
    pub asks: Vec<[f64; 2]>,
}
