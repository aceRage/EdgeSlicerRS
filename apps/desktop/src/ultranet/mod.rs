//! Printing to Bambu Lab printers on the local network through UltraNet,
//! the owner's own network plug-in, run as a separate process (the
//! bridge). ULTRANET_BRIDGE.md is the contract; this is the app's side of
//! it. Nothing here runs unless the person asks for something that needs a
//! printer, and nothing looks for printers on the network.

pub mod client;
pub mod link;
pub mod machine;
pub mod mapping;
#[cfg(test)]
pub mod mock;
pub mod printers;
pub mod send;
#[cfg(test)]
mod tests;
