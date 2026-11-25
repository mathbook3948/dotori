pub mod app;
pub mod card;
pub mod discord;
pub mod slack;
pub mod telegram;

/// The channel the results of the calls the owner allowed or declined come in on. They are data
/// from a tool, not the owner's words.
pub const DECISION: &str = "decision";
