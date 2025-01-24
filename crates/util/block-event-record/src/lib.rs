mod block;
mod event;
mod recorder;

use block::BlockRecord;
use cfx_types::H256;
use recorder::Recorder;

pub use event::Event;

pub fn record_event(hash: &H256, event: Event) {
    let mut recorder = Recorder::get_instance_mut();
    recorder.record_event(hash, event);
}

pub fn record_custom_event(hash: &H256, name: &'static str, stage: usize) {
    let mut recorder = Recorder::get_instance_mut();
    recorder.record_custom_event(hash, name, stage);
}

pub fn record_custom_gauge(hash: &H256, name: &'static str, value: u64) {
    let mut recorder = Recorder::get_instance_mut();
    recorder.record_custom_gauge(hash, name, value);
}

pub fn record_event_for_blocks(
    hashes: impl Iterator<Item = H256>, event: Event,
) {
    let mut recorder = Recorder::get_instance_mut();
    for hash in hashes {
        recorder.record_event(&hash, event);
    }
}

pub fn new_received_block_hashes(hashes: &[H256]) {
    let mut recorder = Recorder::get_instance_mut();
    for hash in hashes {
        recorder.insert_new_block(hash, BlockRecord::new_received_block());
    }
}

pub fn new_mined_block(hash: &H256) {
    let mut recorder = Recorder::get_instance_mut();
    recorder.insert_new_block(hash, BlockRecord::new_mined_block());
}

pub fn handle_shutdown() {
    let mut recorder = Recorder::get_instance_mut();
    for (hash, record) in recorder.peek_iter() {
        record.log(hash);
    }
    recorder.clear();
}
