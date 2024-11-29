use crate::{
    executive::frame::Resumable,
    vm::{self, ActionParams},
};

pub enum InternalTrapResult<T> {
    Return(vm::Result<T>),
    Invoke(ActionParams, Box<dyn Resumable>),
}
use InternalTrapResult::*;

impl<T> InternalTrapResult<T> {
    pub fn map_return<F, U>(self, f: F) -> InternalTrapResult<U>
    where F: FnOnce(T) -> vm::Result<U> {
        match self {
            Return(Ok(r)) => Return(f(r)),
            Return(Err(e)) => Return(Err(e)),
            Invoke(p, r) => Invoke(p, r),
        }
    }
}
