use crate::event::{CustomEvent, CustomGauge};

use super::event::Event;

use cfx_types::H256;
use log::debug;
use std::{
    collections::BTreeMap,
    fmt::Debug,
    time::{Instant, SystemTime},
};

pub(crate) struct BlockRecord {
    hash_ready_instant: Instant,
    hash_ready_time: SystemTime,
    event_ticks: BTreeMap<Event, u64>,
    custom_event_ticks: BTreeMap<CustomEvent, u64>,
    custom_gauges: BTreeMap<CustomGauge, u64>,
}

impl BlockRecord {
    pub(crate) fn new_received_block() -> Self {
        let hash_ready_time = SystemTime::now();
        let hash_ready_instant = Instant::now();
        Self {
            hash_ready_time,
            hash_ready_instant,
            event_ticks: BTreeMap::new(),
            custom_event_ticks: BTreeMap::new(),
            custom_gauges: BTreeMap::new(),
        }
    }

    pub(crate) fn new_mined_block() -> Self {
        use Event::{BodyReady, HeaderReady};
        let mut record = Self::new_received_block();
        record.event_ticks.insert(HeaderReady, 0);
        record.event_ticks.insert(BodyReady, 0);
        record
    }

    pub(crate) fn record_event(&mut self, event: Event) -> bool {
        self.event_ticks.entry(event).or_insert_with(|| {
            self.hash_ready_instant.elapsed().as_nanos() as u64
        });

        self.is_complete()
    }

    pub(crate) fn record_custom_event(
        &mut self, name: &'static str, stage: usize,
    ) {
        let custom_event = CustomEvent::new(name, stage);
        self.custom_event_ticks
            .entry(custom_event)
            .or_insert_with(|| {
                self.hash_ready_instant.elapsed().as_nanos() as u64
            });
    }

    pub(crate) fn record_custom_gauge(
        &mut self, name: &'static str, value: u64,
    ) {
        let custom_gauge = CustomGauge::new(name);
        self.custom_gauges.entry(custom_gauge).or_insert(value);
    }

    pub(crate) fn log(&self, hash: &H256) {
        use chrono::{DateTime, Utc};
        let start_time: DateTime<Utc> = self.hash_ready_time.into();

        let mut result = vec![
            dict_item("hash", hash),
            dict_item("start_timestamp", start_time),
        ];
        result.extend(
            self.event_ticks
                .iter()
                .map(|(event, value)| dict_item(event.key(), value)),
        );
        result.extend(self.custom_event_ticks.iter().map(
            |(custom_event, value)| dict_item(&custom_event.key(), value),
        ));
        result.extend(self.custom_gauges.iter().map(
            |(custom_gauge, value)| dict_item(&custom_gauge.key(), value),
        ));

        let status = if self.is_complete() {
            "complete"
        } else if self.is_complete_for_non_pivot() {
            "partially complete"
        } else {
            "incomplete"
        };

        debug!("Block events record {status}. {}", result.join(", "));
    }

    fn is_complete(&self) -> bool { self.event_ticks.len() == 8 }

    fn is_complete_for_non_pivot(&self) -> bool {
        self.event_ticks.range(..=Event::ConGraphDone).count() == 5
    }
}

fn dict_item(key: &str, value: impl Debug) -> String {
    format!("{key}: {value:?}")
}
