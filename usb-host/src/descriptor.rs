//! A collection of structures defining descriptors in the USB.
//!
//! These types are defined in §9.5 and 9.6 of the USB 2.0
//! specification.
//!
//! The structures defined herein are `repr(C)` and `repr(packed)`
//! when necessary to ensure that they are able to be directly
//! marshalled to the bus.

use core::mem;
use crate::{ENDPOINT_DIRECTION_MASK};

#[derive(Clone, Copy, Debug, PartialEq, defmt::Format, strum_macros::FromRepr)]
#[repr(u8)]
pub enum DescriptorType {
    Device = 1,
    Configuration = 2,
    String = 3,
    Interface = 4,
    Endpoint = 5,

    DeviceQualifier = 6,
    OtherSpeed = 7,
    InterfacePower = 8,
    OTG = 9,
    Debug = 0xA,
    InterfaceAssociation = 0xB,

    // for wireless
    Security = 0xC,
    Key = 0xD,
    EncryptionType = 0xE,
    WirelessEndpointComp = 0x11,

    // for superspeed, wireless and link-power management
    BinaryObjectStore = 0xF,
    DeviceCapability = 0x10,

    ClassInterface = 0x24,
    ClassEndpoint = 0x25,

    SuperSpeedEndpointComp = 0x30,
}

#[derive(Clone, Copy, Debug, PartialEq, defmt::Format, strum_macros::FromRepr)]
#[repr(u8)]
pub enum Class {
    FromInterface = 0x0,
    Audio = 0x01,
    Cdc = 0x02,
    Hid = 0x03,
    Physical = 0x05,
    Imaging = 0x06,
    Printer = 0x07,
    MassStorage = 0x08,
    Hub = 0x09,
    CdcData = 0x0A,
    SmartCard = 0x0B,
    ContentSecurity = 0x0D,
    Video = 0x0E,
    PersonalHealthcare = 0x0F,
    AudioVideo = 0x10,
    Billboard = 0x11,
    UsbTypeCBridge = 0x12,
    I3C = 0x30,
    Diagnostic = 0xDC,
    WirelessController = 0xE0,
    Misc = 0xEF,
    ApplicationSpecific = 0xFE,
    VendorSpecific = 0xFF,
}

#[derive(Copy, Clone, Debug, PartialEq, defmt::Format)]
#[repr(C)]
pub struct DeviceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: DescriptorType,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

impl Default for DeviceDescriptor {
    fn default() -> Self {
        Self {
            b_length: mem::size_of::<Self>() as u8,
            b_descriptor_type: DescriptorType::Device,
            bcd_usb: 0,
            b_device_class: 0,
            b_device_sub_class: 0,
            b_device_protocol: 0,
            b_max_packet_size: 0,
            id_vendor: 0,
            id_product: 0,
            bcd_device: 0,
            i_manufacturer: 0,
            i_product: 0,
            i_serial_number: 0,
            b_num_configurations: 0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, defmt::Format)]
#[repr(C)]
pub struct ConfigurationDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: DescriptorType,
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
}

impl Default for ConfigurationDescriptor {
    fn default() -> Self {
        Self {
            b_length: mem::size_of::<Self>() as u8,
            b_descriptor_type: DescriptorType::Configuration,
            w_total_length: 0,
            b_num_interfaces: 0,
            b_configuration_value: 0,
            i_configuration: 0,
            bm_attributes: 0,
            b_max_power: 0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, defmt::Format)]
#[repr(C)]
pub struct InterfaceAssociationDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: DescriptorType,
    pub b_first_interface: u8,
    pub b_interface_count: u8,
    pub b_function_class: u8,
    pub b_function_sub_class: u8,
    pub b_function_protocol: u8,
    pub i_function: i8,
}

#[derive(Copy, Clone, Debug, PartialEq, defmt::Format)]
#[repr(C)]
pub struct InterfaceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: DescriptorType,
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
}

#[derive(Copy, Clone, Debug, PartialEq, defmt::Format)]
#[repr(C)]
pub struct EndpointDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: DescriptorType,
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

