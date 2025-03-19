extern crate lazy_static;
extern crate metrics;
extern crate once_cell;

use lazy_static::lazy_static;
use metrics::{register_meter_with_group, Histogram, Meter, Sample};
use once_cell::sync::Lazy;
use std::sync::Arc;

lazy_static! {
    pub static ref P2P_RECEIVE_BYTES: MessageMeter =
        MessageMeter::new(|msg_name| {
            register_meter_with_group(
                "p2p_events",
                &format!("{msg_name}_receive_bytes"),
            )
        });
    pub static ref P2P_RECEIVE_CNT: MessageMeter =
        MessageMeter::new(|msg_name| {
            register_meter_with_group(
                "p2p_events",
                &format!("{msg_name}_receive_count"),
            )
        });
    pub static ref P2P_RECEIVE_DECODE_TIME: MessageMeter =
        MessageMeter::new(|msg_name| {
            Sample::ExpDecay(0.015).register_with_group(
                "p2p_events",
                &format!("{msg_name}_decode_time"),
                4096,
            )
        });
    pub static ref P2P_REQUEST_TIME: MessageMeter =
        MessageMeter::new(|msg_name| {
            Sample::ExpDecay(0.015).register_with_group(
                "p2p_events",
                &format!("{msg_name}_request_time"),
                4096,
            )
        });
    pub static ref P2P_RECEIVE_PROCESS_TIME: MessageMeter =
        MessageMeter::new(|msg_name| {
            Sample::ExpDecay(0.015).register_with_group(
                "p2p_events",
                &format!("{msg_name}_process_time"),
                4096,
            )
        });
    pub static ref P2P_SEND_BYTES: MessageMeter =
        MessageMeter::new(|msg_name| {
            register_meter_with_group(
                "p2p_events",
                &format!("{msg_name}_send_bytes"),
            )
        });
    pub static ref P2P_SEND_CNT: MessageMeter = MessageMeter::new(|msg_name| {
        register_meter_with_group(
            "p2p_events",
            &format!("{msg_name}_send_count"),
        )
    });
    pub static ref P2P_QUEUE_WAIT_TIME: MessageMeter =
        MessageMeter::new(|msg_name| {
            Sample::ExpDecay(0.015).register_with_group(
                "p2p_events",
                &format!("{msg_name}_queue_wait_time"),
                4096,
            )
        });
    pub static ref P2P_SEND_WAIT_TIME: MessageMeter =
        MessageMeter::new(|msg_name| {
            Sample::ExpDecay(0.015).register_with_group(
                "p2p_events",
                &format!("{msg_name}_send_wait_time"),
                4096,
            )
        });
}

pub trait MeterRecord: Send + Sync + 'static {
    fn record(&self, v: u64);
}

impl MeterRecord for Arc<dyn Meter> {
    fn record(&self, v: u64) { self.mark(v as usize) }
}

impl MeterRecord for Arc<dyn Histogram> {
    fn record(&self, v: u64) { self.update(v) }
}

impl<T: MeterRecord, F: Send + 'static + Fn() -> T> MeterRecord for Lazy<T, F> {
    fn record(&self, v: u64) { Lazy::force(&self).record(v) }
}

pub struct MessageMeter {
    all: Box<dyn MeterRecord>,
    other: Box<dyn MeterRecord>,
    inner: [Box<dyn MeterRecord>; 36],
}

impl MessageMeter {
    pub fn new<
        F: Copy + Send + 'static + Fn(&'static str) -> T,
        T: MeterRecord,
    >(
        f: F,
    ) -> Self {
        Self {
            all: { Box::new(Lazy::new(move || f("all_msg"))) },
            other: { Box::new(Lazy::new(move || f("other_msg"))) },
            inner: std::array::from_fn(|idx| -> Box<dyn MeterRecord> {
                Box::new(Lazy::new(move || f(MSG_NAME[idx])))
            }),
        }
    }

    pub fn mark(&self, msg_id: u16, num: u64) {
        self.all.record(num);

        let meter = if let Some(m) = msg_id
            .checked_sub(1)
            .and_then(|i| self.inner.get(i as usize))
        {
            &**m
        } else {
            &*self.other
        };

        meter.record(num);
    }
}

const MSG_NAME: [&'static str; 36] = [
    "new_block_hashes",
    "transactions",
    "get_block_hashes",
    "get_block_hashes_response",
    "get_block_headers",
    "get_block_headers_response",
    "get_block_bodies",
    "get_block_bodies_response",
    "new_block",
    "get_terminal_block_hashes_response",
    "get_terminal_block_hashes",
    "get_blocks",
    "get_blocks_response",
    "get_blocks_with_public_response",
    "get_cmpct_blocks",
    "get_cmpct_blocks_response",
    "get_block_txn",
    "get_block_txn_response",
    "dynamic_capability_change",
    "transaction_digests",
    "get_transactions",
    "get_transactions_response",
    "get_block_hashes_by_epoch",
    "get_block_header_chain",
    "get_snapshot_manifest",
    "get_snapshot_manifest_response",
    "get_snapshot_chunk",
    "get_snapshot_chunk_response",
    "get_transactions_from_tx_hashes",
    "get_transactions_from_tx_hashes_response",
    "",
    "state_sync_candidate_request",
    "state_sync_candidate_response",
    "status_v2",
    "status_v3",
    "heartbeat",
];
