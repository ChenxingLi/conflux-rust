use super::{
    internal_transfer::AddressPocket, CallTracer, CheckpointTracer, DrainTrace,
    InternalTransferTracer,
};
use crate::{
    frame::{FrameResult, FrameReturn},
    observer::trace::{
        Action, Call, CallResult, Create, CreateResult, ExecTrace,
        InternalTransferAction,
    },
};
use cfx_types::U256;
use cfx_vm_types::ActionParams;
use typemap::ShareDebugMap;

/// Simple executive tracer. Traces all calls and creates.
#[derive(Default)]
pub struct ExecutiveTracer {
    traces: Vec<Action>,
    valid_indices: CheckpointLog<usize>,
}

impl DrainTrace for ExecutiveTracer {
    fn drain_trace(self, map: &mut ShareDebugMap) {
        map.insert::<TransactionTrace>(self.drain());
    }
}

pub struct TransactionTrace;
impl typemap::Key for TransactionTrace {
    type Value = Vec<ExecTrace>;
}

impl InternalTransferTracer for ExecutiveTracer {
    fn trace_internal_transfer(
        &mut self, from: AddressPocket, to: AddressPocket, value: U256,
    ) {
        let action = Action::InternalTransferAction(InternalTransferAction {
            from,
            to,
            value,
        });

        self.valid_indices.push(self.traces.len());
        self.traces.push(action);
    }
}

impl CheckpointTracer for ExecutiveTracer {
    fn trace_checkpoint(&mut self) { self.valid_indices.checkpoint(); }

    fn trace_checkpoint_discard(&mut self) {
        self.valid_indices.discard_checkpoint();
    }

    fn trace_checkpoint_revert(&mut self) {
        self.valid_indices.revert_checkpoint();
    }
}

impl CallTracer for ExecutiveTracer {
    fn record_call(&mut self, params: &ActionParams) {
        let action = Action::Call(Call::from(params.clone()));

        self.valid_indices.checkpoint();
        self.valid_indices.push(self.traces.len());

        self.traces.push(action);
    }

    fn record_call_result(&mut self, result: &FrameResult) {
        let action = Action::CallResult(CallResult::from(result));
        let success = matches!(
            result,
            Ok(FrameReturn {
                apply_state: true, ..
            })
        );

        self.valid_indices.push(self.traces.len());
        self.traces.push(action);
        if success {
            self.valid_indices.discard_checkpoint();
        } else {
            self.valid_indices.revert_checkpoint();
        }
    }

    fn record_create(&mut self, params: &ActionParams) {
        let action = Action::Create(Create::from(params.clone()));

        self.valid_indices.checkpoint();
        self.valid_indices.push(self.traces.len());
        self.traces.push(action);
    }

    fn record_create_result(&mut self, result: &FrameResult) {
        let action = Action::CreateResult(CreateResult::from(result));
        let success = matches!(
            result,
            Ok(FrameReturn {
                apply_state: true, ..
            })
        );

        self.valid_indices.push(self.traces.len());
        self.traces.push(action);
        if success {
            self.valid_indices.discard_checkpoint();
        } else {
            self.valid_indices.revert_checkpoint();
        }
    }
}

impl ExecutiveTracer {
    pub fn drain(self) -> Vec<ExecTrace> {
        let mut validity: Vec<bool> = vec![false; self.traces.len()];
        for index in self.valid_indices.drain() {
            validity[index] = true;
        }
        self.traces
            .into_iter()
            .zip(validity.into_iter())
            .map(|(action, valid)| ExecTrace { action, valid })
            .collect()
    }
}

#[derive(Default)]
struct CheckpointLog<T> {
    data: Vec<T>,
    checkpoints: Vec<usize>,
}

impl<T> CheckpointLog<T> {
    fn push(&mut self, item: T) { self.data.push(item); }

    fn checkpoint(&mut self) { self.checkpoints.push(self.data.len()); }

    fn revert_checkpoint(&mut self) {
        let start = self.checkpoints.pop().unwrap();
        self.data.truncate(start);
    }

    fn discard_checkpoint(&mut self) { self.checkpoints.pop().unwrap(); }

    fn drain(self) -> Vec<T> { self.data }
}