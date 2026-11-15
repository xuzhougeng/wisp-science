//! Computational method search, a port of the WebView's
//! `method_search.rs` and `method_search_coordinator.rs`: an agent freezes
//! an evaluation contract (`prepare`), then a structured Run proposes
//! candidate implementations with the model, evaluates each with the frozen
//! evaluator, and promotes the best.

pub(crate) mod prepare;
