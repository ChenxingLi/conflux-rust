#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub enum Event {
    HeaderReady,
    BodyReady,
    SyncGraph,
    ConGraph,
    ConGraphDone,
    ComputeEpoch,
    NotifyTxPool,
    TxPoolUpdated,
}

impl Event {
    pub fn key(&self) -> &'static str {
        match self {
            Event::HeaderReady => "header_ready",
            Event::BodyReady => "body_ready",
            Event::SyncGraph => "sync_graph",
            Event::ConGraph => "consensys_graph_insert",
            Event::ConGraphDone => "consensys_graph_ready",
            Event::ComputeEpoch => "compute_epoch",
            Event::NotifyTxPool => "notify_tx_pool",
            Event::TxPoolUpdated => "tx_pool_updated",
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub(crate) struct CustomEvent(&'static str, usize);

impl CustomEvent {
    pub fn new(name: &'static str, stage: usize) -> Self {
        CustomEvent(name, stage)
    }

    pub fn key(&self) -> String { format!("custom_{}_{}", self.0, self.1) }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
pub(crate) struct CustomGauge(&'static str);

impl CustomGauge {
    pub fn new(name: &'static str) -> Self { CustomGauge(name) }

    pub fn key(&self) -> String { format!("gauge_{}", self.0) }
}
