use std::any::Any;
use std::panic;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::BoxFuture;

/// Runs a port call and turns a panic into an `Err` with the panic message, whether the port
/// panics while building its future or while it runs. A broken connector costs one result,
/// never the agent loop.
pub(crate) enum Caught<'a, T> {
    Running(BoxFuture<'a, T>),
    Panicked(Option<String>),
}

pub(crate) fn caught<'a, T>(start: impl FnOnce() -> BoxFuture<'a, T>) -> Caught<'a, T> {
    match panic::catch_unwind(AssertUnwindSafe(start)) {
        Ok(future) => Caught::Running(future),
        Err(payload) => Caught::Panicked(Some(panic_message(payload.as_ref()))),
    }
}

impl<T> Future for Caught<'_, T> {
    type Output = Result<T, String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Caught::Running(future) => {
                match panic::catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
                    Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
                    Ok(Poll::Pending) => Poll::Pending,
                    Err(payload) => Poll::Ready(Err(panic_message(payload.as_ref()))),
                }
            }
            Caught::Panicked(message) => Poll::Ready(Err(message
                .take()
                .expect("a finished future is not polled again"))),
        }
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message");
    format!("panicked: {message}")
}
