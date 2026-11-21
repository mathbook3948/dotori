use super::*;

struct Harness {
    state: AgentState,
    effects: Vec<Effect>,
}

impl Harness {
    fn new() -> Self {
        todo!()
    }

    fn owner_says(&mut self, text: &str) {
        todo!()
    }

    fn other_says(&mut self, place: &str, text: &str) {
        todo!()
    }

    fn model_answers(&mut self, actions: Vec<Action>) {
        todo!()
    }

    fn tool_returns(&mut self, result: Result<String, ToolError>) {
        todo!()
    }

    fn shut_down(&mut self) {
        todo!()
    }
}

#[test]
#[ignore = "sketch"]
fn owner_message_cancels_the_call_in_flight_and_calls_again() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn other_message_lets_the_reply_run_then_calls_again() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn owner_message_waits_for_running_tools() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn result_of_a_cancelled_call_is_ignored() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn redelivered_message_is_heard_once() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn model_failure_is_recorded_and_ends_the_turn() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn delivery_failure_is_recorded_without_calling_the_model() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn shutdown_waits_for_running_tools() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn app_window_shows_traffic_of_other_channels() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn reply_to_an_unknown_thread_is_refused() {
    todo!()
}

#[test]
#[ignore = "sketch"]
fn sub_task_outcome_joins_the_next_call() {
    todo!()
}
