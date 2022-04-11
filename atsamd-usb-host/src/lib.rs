//! USB Host driver implementation for SAMD* series chips.
//! Refer to Atmel SMART SAM SD21 Datasheet for detailed explanation of registers and shit

#![no_std]

extern crate alloc;

#[macro_use]
extern crate defmt;

mod host;
mod pipe;
mod error;

pub use host::{HostEvent, SAMDHost, Pins};
