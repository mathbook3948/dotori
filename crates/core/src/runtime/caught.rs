use std::any::Any;
use std::panic;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use crate::BoxFuture;

/// Runs a port future and turns a panic inside it into an `Err` with the panic message,
/// so a broken connector costs one result instead of the agent loop.
pub(crate) struct Caught<'a, T>(BoxFuture<'a, T>);

pub(crate) fn caught<T>(future: BoxFuture<'_, T>) -> Caught<'_, T> {
    Caught(future)
}

impl<T> Future for Caught<'_, T> {
    type Output = Result<T, String>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match panic::catch_unwind(AssertUnwindSafe(|| self.0.as_mut().poll(cx))) {
            Ok(Poll::Ready(value)) => Poll::Ready(Ok(value)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(payload) => Poll::Ready(Err(panic_message(payload.as_ref()))),
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
