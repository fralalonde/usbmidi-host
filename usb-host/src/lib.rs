//! This crate defines a set of traits for use on the host side of the
//! USB.
//!
//! The `USBHost` defines the Host Controller Interface that can be
//! used by the `Driver` interface.
//!
//! The `Driver` interface defines the set of functions necessary to
//! use devices plugged into the host.

#![no_std]

// required for async trait
extern crate alloc;

#[macro_use]
extern crate defmt;

use async_trait::async_trait;
use alloc::boxed::Box;

pub mod descriptor;
pub mod control;
pub mod device;
pub mod address;
pub mod parser;
pub mod class;

use core::convert::TryFrom;
use core::fmt::{Debug, Formatter};
use core::mem;
pub use descriptor::*;
pub use control::*;
use crate::address::Address;
use crate::device::Device;
use crate::parser::DescriptorParser;

/// Errors that can be generated when attempting to do a USB transfer.
#[derive(Debug, defmt::Format)]
pub enum TransferError {
    /// An error that may be retried.
    Retry(&'static str),

    /// A permanent error.
    Permanent(&'static str),

    Runtime(runtime::RuntimeError),
    InvalidDescriptor,
    TooManyJacks,
    EnumerationFailed,
}

impl From<runtime::RuntimeError> for TransferError {
    fn from(err: runtime::RuntimeError) -> Self {
        TransferError::Runtime(err)
    }
}

/// Trait for host controller interface.
#[async_trait]
pub trait USBHost: Sync + Send {
    async fn get_host_id(&self) -> u8;

    /// Issue a control transfer with an optional data stage to `ep`
    /// The data stage direction is determined by the direction of `bm_request_type`
    ///
    /// On success, the amount of data transferred into `buf` is returned.
    async fn control_transfer(&mut self, ep: &dyn Endpoint, bm_request_type: RequestType, b_request: RequestCode, w_value: WValue, w_index: u16, buf: Option<&mut [u8]>) -> Result<usize, TransferError>;

    /// Issue a transfer from `ep` to the host
    /// On success, the amount of data transferred into `buf` is returned
    async fn in_transfer(&mut self, ep: &dyn Endpoint, buf: &mut [u8]) -> Result<usize, TransferError>;

    /// Issue a transfer from the host to `ep`
    /// All buffer is sent or transfer fails
    async fn out_transfer(&mut self, ep: &dyn Endpoint, buf: &[u8]) -> Result<usize, TransferError>;
}

/// The type of transfer to use when talking to USB devices.
///
/// cf §9.6.6 of USB 2.0
#[derive(Copy, Clone, Debug, PartialEq, strum_macros::FromRepr, defmt::Format)]
#[repr(u8)]
pub enum TransferType {
    Control = 0x0,
    Isochronous = 0x1,
    Bulk = 0x2,
    Interrupt = 0x3,
}

/// The direction of the transfer with the USB device.
///
/// cf §9.6.6 of USB 2.0
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Direction {
    Out,
    In,
}

pub fn to_slice_mut<T>(v: &mut T) -> &mut [u8] {
    let ptr = v as *mut T as *mut u8;
    unsafe { core::slice::from_raw_parts_mut(ptr, mem::size_of::<T>()) }
}

/// Bit 7 is the direction, with OUT = 0 and IN = 1
const ENDPOINT_DIRECTION_MASK: u8 = 0x80;

/// Bits 3..0 are the endpoint number
const ENDPOINT_NUMBER_MASK: u8 = 0x0F;

/// `Endpoint` defines the USB endpoint for various transfers.
pub trait Endpoint: Debug + Send + Sync {
    /// Address of the device owning this endpoint
    fn device_address(&self) -> Address;

    /// Endpoint address, unique for the interface (includes direction bit)
    fn endpoint_address(&self) -> u8;

    /// Direction inferred from endpoint address
    fn direction(&self) -> Direction {
        match self.endpoint_address() & ENDPOINT_DIRECTION_MASK  {
            0 => Direction::Out,
            _ => Direction::In
        }
    }

    /// Endpoint number, irrespective of direction
    /// Two endpoints per interface can share the same number (one IN, one OUT)
    fn endpoint_num(&self) -> u8 {
        self.endpoint_address() & ENDPOINT_NUMBER_MASK
    }

    /// The type of transfer this endpoint uses
    fn transfer_type(&self) -> TransferType;

    /// The maximum packet size for this endpoint
    fn max_packet_size(&self) -> u16;
}

#[async_trait]
pub trait ControlEndpoint {
    async fn control_get_descriptor(&self, host: &mut dyn USBHost, desc_type: DescriptorType, idx: u8, buffer: &mut [u8]) -> Result<usize, TransferError>;

    async fn control_set(&self, host: &mut dyn USBHost, param: RequestCode, lo_val: u8, hi_val: u8, index: u16) -> Result<(), TransferError>;
}

#[async_trait]
pub trait BulkEndpoint {
    async fn bulk_in(&self, host: &mut dyn USBHost, buffer: &mut [u8]) -> Result<usize, TransferError>;

    async fn bulk_out(&self, host: &mut dyn USBHost, buffer: &[u8]) -> Result<usize, TransferError>;
}

#[derive(defmt::Format)]
pub struct SingleEp {
    pub device_address: Address,
    pub endpoint_address: u8,
    pub transfer_type: TransferType,
    pub max_packet_size: u16,
}

impl Debug for SingleEp {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.endpoint_num().fmt(f)?;
        self.direction().fmt(f)?;
        self.transfer_type().fmt(f)
    }
}

impl TryFrom<(Address, &EndpointDescriptor)> for SingleEp {
    type Error = TransferError;

    fn try_from(addr_ep_desc: (Address, &EndpointDescriptor)) -> Result<Self, Self::Error> {
        Ok(SingleEp {
            device_address: addr_ep_desc.0.into(),
            endpoint_address: addr_ep_desc.1.b_endpoint_address,
            transfer_type: TransferType::from_repr(addr_ep_desc.1.bm_attributes).ok_or(TransferError::InvalidDescriptor)?,
            max_packet_size: addr_ep_desc.1.w_max_packet_size,
        })
    }
}

impl Endpoint for SingleEp {
    fn device_address(&self) -> Address {
        self.device_address
    }

    fn endpoint_address(&self) -> u8 {
        self.endpoint_address
    }

    fn transfer_type(&self) -> TransferType {
        self.transfer_type
    }

    fn max_packet_size(&self) -> u16 {
        self.max_packet_size
    }
}

#[async_trait]
impl BulkEndpoint for SingleEp {
    async fn bulk_in(&self, host: &mut dyn USBHost, buffer: &mut [u8]) -> Result<usize, TransferError> {
        todo!()
    }

    async fn bulk_out(&self, host: &mut dyn USBHost, buffer: &[u8]) -> Result<usize, TransferError> {
        todo!()
    }
}


/// Types of errors that can be returned from a `Driver`.
#[derive(Copy, Clone, Debug, defmt::Format)]
pub enum DriverError {
    /// An error that may be retried.
    Retry(u8, &'static str),

    /// A permanent error.
    Permanent(u8, &'static str),
}


/// Trait for drivers on the USB host.
#[async_trait]
pub trait Driver: core::fmt::Debug {
    async fn connected(&mut self, host: &mut dyn USBHost, device: &mut Device, device_desc: &DeviceDescriptor, config_descriptors: &mut DescriptorParser) -> Result<bool, TransferError>;

    async fn disconnected(&mut self, host: &mut dyn USBHost, device: &mut Device);

    async fn tick(&mut self, host: &mut dyn USBHost) -> Result<(), DriverError>;
}
