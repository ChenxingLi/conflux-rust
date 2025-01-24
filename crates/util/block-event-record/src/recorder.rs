use std::{
    ops::{Deref, DerefMut},
    time::Instant,
};

use super::{block::BlockRecord, event::Event};

use cfx_types::H256;
use lazy_static::lazy_static;
use log::{debug, warn};
use lru_time_cache::LruCache;
use parking_lot::Mutex;

pub(crate) struct Recorder {
    lock_acquire_time: u128,
    records: LruCache<H256, BlockRecord>,
}

lazy_static! {
    pub(crate) static ref BLOCK_EVENT_RECORDER: Mutex<Recorder> =
        Mutex::new(Recorder::with_capacity(4000));
}

impl Deref for Recorder {
    type Target = LruCache<H256, BlockRecord>;

    fn deref(&self) -> &Self::Target { &self.records }
}

impl DerefMut for Recorder {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.records }
}

impl Recorder {
    fn with_capacity(capacity: usize) -> Self {
        Recorder {
            records: LruCache::with_capacity(capacity),
            lock_acquire_time: 0,
        }
    }

    pub(crate) fn get_instance_mut() -> impl DerefMut<Target = Recorder> {
        let start = Instant::now();
        let mut recorder = BLOCK_EVENT_RECORDER.lock();
        recorder.lock_acquire_time += start.elapsed().as_nanos();

        recorder
    }

    pub fn record_event(&mut self, hash: &H256, event: Event) {
        let block_record = if let Some(r) = self.records.get_mut(hash) {
            r
        } else {
            warn!(
                "Block events record error: unknown hash {:?} on event {:?}",
                hash, event
            );
            return;
        };

        let record_completed = block_record.record_event(event);

        if record_completed {
            block_record.log(hash);
            self.records.remove(&hash);
        }
    }

    pub fn record_custom_event(
        &mut self, hash: &H256, name: &'static str, stage: usize,
    ) {
        let block_record = if let Some(r) = self.records.get_mut(hash) {
            r
        } else {
            warn!(
                "Block events record error: unknown hash {:?} on custom event {} at stage {}",
                hash, name, stage
            );
            return;
        };

        block_record.record_custom_event(name, stage);
    }

    pub fn record_custom_gauge(
        &mut self, hash: &H256, name: &'static str, value: u64,
    ) {
        let block_record = if let Some(r) = self.records.get_mut(hash) {
            r
        } else {
            warn!(
                "Block gauge record error: unknown hash {:?} on custom gauge {}",
                hash, name
            );
            return;
        };

        block_record.record_custom_gauge(name, value);
    }

    pub fn insert_new_block(&mut self, hash: &H256, record: BlockRecord) {
        if self.records.contains_key(hash) {
            return;
        }
        let (_, eliminated_items) = self.records.notify_insert(*hash, record);

        Self::process_eliminated_items(eliminated_items)
    }

    pub fn clear(&mut self) {
        debug!(
            "Recorder statistics. recorder_lock_wait: {}",
            self.lock_acquire_time
        );
        self.records.clear();
        self.lock_acquire_time = 0;
    }

    fn process_eliminated_items(
        items: impl IntoIterator<Item = (H256, BlockRecord)>,
    ) {
        for (hash, record) in items {
            record.log(&hash)
        }
    }
}
