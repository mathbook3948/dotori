//! A small JavaScript interpreter for the Playwright-style code the model writes to drive the
//! browser. It runs only what such code needs. Code meant for inside the page runs in the page.

pub(crate) mod lex;
pub(crate) mod parse;
pub(crate) mod run;
