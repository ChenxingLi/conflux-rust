use lazy_static::lazy_static;
use metrics::{
    register_adv_timer_with_group, AdvancedTimer, Gauge, GaugeUsize,
};
use std::sync::Arc;

lazy_static! {
    pub static ref PACKED_ACCOUNT_SIZE: Arc<dyn Gauge<usize>> =
        GaugeUsize::register_with_group("txpool", "packed_account_size");
    pub static ref GENERATE_ONE_BLOCK: AdvancedTimer =
        register_adv_timer_with_group("blockgen", "generate");
    pub static ref PACK_TRANSACTION: AdvancedTimer =
        register_adv_timer_with_group("blockgen", "pack_transaction");
    pub static ref GET_BLAME: AdvancedTimer =
        register_adv_timer_with_group("blockgen", "get_blame");
    pub static ref ASSEMBLE_BLOCK: AdvancedTimer =
        register_adv_timer_with_group("blockgen", "assemble_block");
    pub static ref RESOLVE_PUZZLE: AdvancedTimer =
        register_adv_timer_with_group("blockgen", "resolve_puzzle");
    pub static ref WAIT_GENERATION: AdvancedTimer =
        register_adv_timer_with_group("blockgen", "wait_generation");
}
