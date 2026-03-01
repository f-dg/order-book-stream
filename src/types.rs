use std::{default::Default, time::SystemTime};

#[derive(Clone, Debug)]
pub struct OrderBook {
    pub exchnage: Exchange,
    pub bids: Vec<[f64; 2]>, // [price, qty]
    pub asks: Vec<[f64; 2]>, // [price, qty]
    pub received_at: SystemTime,
}

impl Default for OrderBook {
    fn default() -> Self {
        Self {
            exchnage: Default::default(),
            bids: Default::default(),
            asks: Default::default(),
            received_at: SystemTime::now(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Exchange {
    #[default] Binance,
    Bitstamp,
}

impl std::fmt::Display for Exchange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exchange::Binance => write!(f, "binance"),
            Exchange::Bitstamp => write!(f, "bitstamp"),
        }
    }
}
