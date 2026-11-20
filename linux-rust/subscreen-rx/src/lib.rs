//! The parts of subscreen-rx that touch no hardware, kept in a library so they can be tested on
//! any machine (`cargo test --lib`): the control channel's wire format and the content meter.
pub mod content;
pub mod proto;
