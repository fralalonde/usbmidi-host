//! Simple USB host-side driver for boot protocol keyboards.
use core::fmt::{Debug, Formatter};

use heapless::{Deque, FnvIndexMap, Vec};

use usb_host::{DeviceDescriptor, Direction, Driver, DriverError, Endpoint, InterfaceDescriptor, SingleEp, TransferError, UsbHost};
use usb_host::address::Address;

use usb_host::device::Device;
use usb_host::parser::{DescriptorParser, DescriptorRef};
use midi::{Packet, PacketList, PacketParser, Receive, ReceiveListener, Transmit};
use usb_host::class::audio::{AudioDescriptorRef, MSInJackDescriptor, MSOutJackDescriptor};

// How long to wait before talking to the device again after setting
// its address. cf §9.2.6.3 of USB 2.0
// const SETTLE_DELAY: u64 = 2;

// How many total devices this driver can support.
const MAX_DEVICES: usize = 32;

// And how many endpoints we can support per-device.
const MAX_ENDPOINTS: usize = 2;

pub const USB_MIDI_PACKET_LEN: usize = 4;

pub const USB_CLASS_NONE: u8 = 0x00;
pub const USB_AUDIO_CLASS: u8 = 0x01;
pub const USB_AUDIO_CONTROL_SUBCLASS: u8 = 0x01;
pub const USB_MIDI_STREAMING_SUBCLASS: u8 = 0x03;

fn is_midi_interface(idesc: &InterfaceDescriptor) -> bool {
    idesc.b_interface_class == USB_AUDIO_CLASS
        && idesc.b_interface_sub_class == USB_MIDI_STREAMING_SUBCLASS
        && idesc.b_interface_protocol == 0x00
}

const MAX_PORTS: usize = 8;

#[derive(Debug, Eq, PartialEq)]
struct UsbJackId {
    host_id: u8,
    device_address: Address,
    endpoint_address: u8,
    jack_id: u8,
}

pub type MidiFn = Option<&'static mut (dyn FnMut(PacketList) + Send + Sync)>;

const MAX_EP: usize = 8;

/// A single endpoint can have multiple input jacks
static USB_MIDI_IN_EP: FnvIndexMap<(u8, Address), UsbMidiReadEp, MAX_EP> = FnvIndexMap::new();
static USB_MIDI_PORTS: FnvIndexMap<(u8, Address), UsbMidiWritePort, MAX_PORTS> = FnvIndexMap::new();

// = FnvIndexMap::new();
/// Boot protocol keyboard driver for USB hosts.
#[derive(Default, Debug)]
pub struct UsbMidiDriver {
    devices: FnvIndexMap<(u8, Address), UsbMidiPort, MAX_PORTS>,
}

#[derive(Debug)]
pub struct UsbMidiWritePort {
    ep: SingleEp,
    jack_id: u8,
    buffer: Deque<Packet, 17>,
}

impl Transmit for UsbMidiWritePort {
    fn transmit(&mut self, events: embedded_midi::PacketList) -> Result<(), embedded_midi::MidiError> {
        // FIXME PacketList should implement IntoIterator or just be simplified
        for p in &*events {
            self.buffer.push_front(*p);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct UsbMidiReadEp {
    ep: SingleEp,
    ports: FnvIndexMap<u8, UsbMidiReadPort, 16>,
}

pub struct UsbMidiReadPort {
    // jack_id: u8,
    cb: SpinMutex<Option<&'static mut (dyn FnMut(PacketList) + Send + Sync)>>,
}

impl Default for UsbMidiReadPort {
    fn default() -> Self {
        Self { cb: SpinMutex::new(None) }
    }
}

impl Debug for UsbMidiReadPort {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        // self.ep.fmt(f)
        Ok(())
    }
}

impl ReceiveListener for UsbMidiReadPort {
    fn on_receive(&mut self, listener: Option<&'static mut (dyn FnMut(PacketList) + Send + Sync)>) {
        *self.cb.lock() = listener
    }
}

 impl UsbMidiDriver {
     fn connected(&mut self, host: &mut dyn UsbHost, device: &mut Device, device_desc: &DeviceDescriptor, config_descriptors: &mut DescriptorParser) -> Result<bool, TransferError> {
         let mut config = None;
         let mut midi_interface = None;
         let mut in_ep = None;
         let mut out_ep = None;
         let mut in_jacks: Vec<&MSInJackDescriptor, 16> = Vec::new();
         let mut out_jacks: Vec<&MSOutJackDescriptor, 16> = Vec::new();

         while let Some(desc) = config_descriptors.next() {
             debug!("USB Enum {:?}", desc);
             match desc {
                 DescriptorRef::Configuration(cdesc) =>
                     config = Some(cdesc),

                 DescriptorRef::Interface(idesc) =>
                     if is_midi_interface(idesc) {
                         if midi_interface.is_some() {
                             // new interface, done enumerating MIDI endpoints
                             break;
                         }
                         midi_interface = Some(idesc)
                     }

                 DescriptorRef::Audio(AudioDescriptorRef::MSOutJack(out_jack)) => {
                     out_jacks.push(out_jack);
                 }

                 DescriptorRef::Audio(AudioDescriptorRef::MSInJack(in_jack)) => {
                     in_jacks.push(in_jack);
                 }

                 DescriptorRef::Endpoint(edesc) => {
                     if let Some(interface_num) = midi_interface {
                         let ep = device.endpoint(edesc)?;
                         if ep.direction() == Direction::Out {
                             if out_ep.is_none() {
                                 out_ep = Some(ep)
                             } else {
                                 warn!("More than one MIDI out endpoint")
                             }
                         } else {
                             if in_ep.is_none() {
                                 in_ep = Some(ep)
                             } else {
                                 warn!("More than one MIDI in endpoint")
                             }
                         }
                     }
                 }
                 _ => {}
             }
         }

         if let Some(midi_if) = midi_interface {
             if let Some(cfg) = config {
                 device.set_configuration(host, cfg.b_configuration_value).await?;
                 debug!("USB MIDI Device Configuration Set {}", cfg.b_configuration_value)
             } else {
                 error!("USB MIDI Device not configured");
                 return Ok(false);
             }

             // debug!("using device interface {}[{}]",  midi_if.b_interface_number,  midi_if.b_alternate_setting);
             if let Err(e) = device.set_interface(host, midi_if.b_interface_number, midi_if.b_alternate_setting).await {
                 // should not matter? "Selecting a configuration, by default, also activates the first alternate setting in each interface in that configuration."
                 warn!("USB MIDI Device set interface {}[{}] failed (ignored) {:?}", midi_if.b_interface_number,  midi_if.b_alternate_setting, e)
             }

             if let Some(ep) = in_ep {
                 let mut read = UsbMidiReadEp {
                     ep,
                     ports: FnvIndexMap::new(),
                 };

                 for ij in in_jacks {
                     read.ports.insert(ij.b_jack_id, UsbMidiReadPort::default()).map_err(|_e| TransferError::TooManyJacks)?;
                 }
                 unsafe { USB_MIDI_IN_EP.insert((host.get_host_id().await, device.get_address()), read); }
             }

             // if let Some(ep) = out_ep {
             //     // let mut read = UsbMidiWritePort {
             //     //     ep,
             //     //     ports: FnvIndexMap::new(),
             //     // };
             //
             //     for oj in out_jacks {
             //         let port = UsbMidiWritePort::default()
             //         read.ports.insert(ij.b_jack_id, UsbMidiReadPort::default());
             //     }
             //     unsafe { USB_MIDI_PORTS.insert((host.get_host_id(), device.get_address()), read); }
             // }

             // TODO out_ports
         }
         Ok(false)
     }

     fn disconnected(&mut self, host: &mut dyn USBHost, device: &mut Device) {
         unsafe { USB_MIDI_PORTS.remove(&(host.get_host_id().await, device.get_address())) };
         // TODO clear other indexes (by name, by id, etc.)
     }
 }

impl Driver for UsbMidiDriver {


    fn want_device(&self, device: &DeviceDescriptor) -> bool {
        // TODO
        false
    }

    fn add_device(&mut self, device: DeviceDescriptor, address: u8) -> Result<(), DriverError> {
        // TODO
        Ok(())
    }

    fn remove_device(&mut self, address: u8) {
        // TODO
    }

    fn tick(&mut self, host: &mut dyn UsbHost) -> Result<(), DriverError> {
        // debug!("TICK");
        for port in unsafe { &mut USB_MIDI_PORTS }.values_mut() {
            if let Some(output) = &mut port.output {
                while let Some(packet) = output.buffer.pop() {
                    // TODO send all packets at once
                    if let Err(e) = host.out_transfer(&mut output.ep, packet.payload()) {
                        warn!("USB OUT failed {:?}", e)
                    }
                }
            }
        }

        for port in unsafe { &mut USB_MIDI_IN_EP }.values_mut() {
            // if let Some(input) = &mut port.ep {
            let mut buf = [0; 64];

            match host.in_transfer(&mut port.ep, &mut buf).await {
                Ok(0) => {
                    debug!("NO DATA")
                }
                Ok(len) => {
                    let mut pp = PacketParser::default();
                    for b in &buf[..len] {
                        match pp.advance(*b) {
                            // TODO receive all packets at once
                            Ok(Some(packet)) => {
                                debug!("PACKET from jack {:?}", packet.cable_number() );
                                if let Some(jack) = port.ports.get(&(packet.cable_number() as u8)) {
                                    if let Some(mut callback) = jack.cb.lock().as_mut() {
                                        (callback)(PacketList::single(packet))
                                    }
                                }
                            }
                            Err(e) => warn!("USB MIDI Packet Error{:?}", e),
                            _ => {}
                        }
                    }
                }
                Err(e) => warn!("USB IN Failed {:?}", e),
                _ => {}
            }
        }
        Ok(())
    }
}
