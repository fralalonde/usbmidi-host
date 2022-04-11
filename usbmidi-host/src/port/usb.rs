use usb_device::device::UsbVidPid;
use usb_device::device::{UsbDevice};
use usb_device::prelude::UsbDeviceBuilder;

use usb_device::{
    bus::{InterfaceNumber, UsbBus, UsbBusAllocator},
    class::UsbClass,
    descriptor::DescriptorWriter,
    endpoint::{EndpointIn, EndpointOut},
    UsbError,
};

use core::result::Result;

#[cfg(feature = "port")]
use hal::otg_fs::{UsbBusType};

use usb_device::class_prelude::EndpointAddress;
use crate::{Packet, MidiError, PacketList};

const USB_TX_BUFFER_SIZE: u16 = 64;

const USB_RX_BUFFER_SIZE: u16 = 64;

// const MIDI_IN_SIZE: u8 = 0x06;
const MIDI_OUT_SIZE: u8 = 0x09;

const USB_CLASS_NONE: u8 = 0x00;
const USB_AUDIO_CLASS: u8 = 0x01;
const USB_AUDIO_CONTROL_SUBCLASS: u8 = 0x01;
const USB_MIDI_STREAMING_SUBCLASS: u8 = 0x03;

const MIDI_IN_JACK_SUBTYPE: u8 = 0x02;
const MIDI_OUT_JACK_SUBTYPE: u8 = 0x03;

const EMBEDDED: u8 = 0x01;
const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;
const HEADER_SUBTYPE: u8 = 0x01;
const MS_HEADER_SUBTYPE: u8 = 0x01;
const MS_GENERAL: u8 = 0x01;

/// Configures the usb devices as seen by the operating system.
pub fn usb_device<B: UsbBus>(usb_bus: &UsbBusAllocator<B>) -> UsbDevice<B> {
    UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
        .manufacturer("M'Roto")
        .product("USB MIDI Router")
        .serial_number("123")
        .device_class(USB_CLASS_NONE)
        .build()
}

const PACKET_LEN: usize = 4;
const TX_FIFO_SIZE: usize = USB_TX_BUFFER_SIZE as usize;
const RX_FIFO_SIZE: usize = USB_RX_BUFFER_SIZE as usize + PACKET_LEN;

pub struct UsbMidi {
    pub dev: UsbDevice<'static, UsbBusType>,
    pub midi_class: MidiClass<'static, UsbBusType>,
}

impl UsbMidi {
    /// USB upkeep
    pub fn poll(&mut self) -> bool {
        self.dev.poll(&mut [&mut self.midi_class])
    }
}

impl crate::Transmit for UsbMidi {
    fn transmit(&mut self, packets: PacketList) -> Result<(), MidiError> {
        for packet in packets.iter() {
            self.midi_class.tx_push(packet.bytes());
        }
        self.midi_class.tx_flush();
        Ok(())
    }
}

impl crate::Receive for UsbMidi {
    fn receive(&mut self) -> Result<Option<Packet>, MidiError> {
        if let Some(bytes) = self.midi_class.receive() {
            return Ok(Some(Packet::from_raw(bytes)));
        }
        Ok(None)
    }
}

///Note we are using MidiIn here to refer to the fact that
///The Host sees it as a midi in devices
///This class allows you to send types in
pub struct MidiClass<'a, B: UsbBus> {
    audio_subclass: InterfaceNumber,
    midi_subclass: InterfaceNumber,

    bulk_out: EndpointOut<'a, B>,
    bulk_in: EndpointIn<'a, B>,

    tx_fifo: [u8; TX_FIFO_SIZE],
    tx_len: usize,

    rx_fifo: [u8; RX_FIFO_SIZE],
    rx_end: usize,
    rx_start: usize,
}

impl<B: UsbBus> MidiClass<'_, B> {
    /// Creates a new MidiClass with the provided UsbBus
    pub fn new(usb_alloc: &UsbBusAllocator<B>) -> MidiClass<'_, B> {
        MidiClass {
            audio_subclass: usb_alloc.interface(),
            midi_subclass: usb_alloc.interface(),

            bulk_out: usb_alloc.bulk(USB_TX_BUFFER_SIZE),
            bulk_in: usb_alloc.bulk(USB_RX_BUFFER_SIZE),

            tx_fifo: [0; TX_FIFO_SIZE],
            tx_len: 0,

            rx_fifo: [0; RX_FIFO_SIZE],
            rx_start: 0,
            rx_end: 0,
        }
    }

    // /// Try enqueue packet, then flush.
    // /// If enqueue failed (because buffer full), retry after flush.
    // /// Drop packet if all else fails.
    // fn send(&mut self, payload: &[u8]) {
    //     let retry_push = !self.tx_push(payload);
    //     let flushed = self.tx_flush();
    //
    //     if retry_push {
    //         if flushed {
    //             // do retry enqueue packet
    //             if !self.tx_push(payload) {
    //                 // but queue was just flushed?! should never happen (famous last words)
    //                 self.tx_drop += 1;
    //             }
    //         } else {
    //             // queue is just as full as before, no sense in retrying
    //             self.tx_drop += 1;
    //         }
    //     }
    // }

    /// Empty TX FIFO to USB devices.
    /// Return true if bytes were sent.
    fn tx_flush(&mut self) -> bool {
        let result = self.bulk_in.write(&self.tx_fifo[0..self.tx_len]);
        match result {
            Ok(count) => {
                self.tx_fifo.copy_within(count..self.tx_len, 0);
                self.tx_len -= count;
                true
            }
            Err(UsbError::WouldBlock) => false,
            Err(err) => panic!("{:?}", err),
        }
    }

    /// Enqueue a packet in TX FIFO
    fn tx_push(&mut self, payload: &[u8]) -> bool {
        if self.tx_len < (TX_FIFO_SIZE - payload.len()) {
            self.tx_fifo[self.tx_len..self.tx_len + payload.len()].copy_from_slice(payload);
            self.tx_len += payload.len();
            return true
        }
        false
    }

    /// Look for buffered bytes
    /// If none, try to get more
    fn receive(&mut self) -> Option<[u8; 4]> {
        if let Some(bytes) = self.rx_pop() {
            Some(bytes)
        } else {
            // FIFO is empty, check USB devices then retry
            self.rx_fill();
            self.rx_pop()
        }
    }

    #[inline]
    fn rx_size(&self) -> usize {
        self.rx_end - self.rx_start
    }

    /// Dequeue a packet from RX FIFO (if any)
    fn rx_pop(&mut self) -> Option<[u8; 4]> {
        if self.rx_size() >= PACKET_LEN {
            let raw = self.rx_fifo.as_chunks().0[0];
            self.rx_start += PACKET_LEN;
            return Some(raw)
        }
        None
    }

    /// Try to fetch packets bytes from USB devices.
    fn rx_fill(&mut self) {
        // compact any odd bytes to buffer start
        self.rx_fifo.copy_within(self.rx_start..self.rx_end, 0);
        self.rx_end = self.rx_size();
        self.rx_start = 0;

        match self.bulk_out.read(&mut self.rx_fifo[self.rx_end..RX_FIFO_SIZE]) {
            Ok(received) => {
                self.rx_end += received;
                assert!(self.rx_end <= self.rx_fifo.len());
            }
            Err(UsbError::WouldBlock) => {}
            Err(err) => panic!("{:?}", err)
        };
    }
}

impl<B: UsbBus> UsbClass<B> for MidiClass<'_, B> {

    /// Callback after USB flush (send) completed
    /// Check for packets that were enqueued while devices was busy (UsbErr::WouldBlock)
    /// If any packets are pending re-flush queue immediately
    /// This callback may chain-trigger under high output load (big sysex, etc.) - this is good
    fn endpoint_in_complete(&mut self, addr: EndpointAddress) {
        if addr == self.bulk_in.address() {
            if self.tx_len > 0 {
                // send pending bytes in tx_buf
                self.tx_flush();
            }
        }
    }

    /// Magic copied from https://github.com/btrepp/rust-midi-stomp (thanks)
    /// For details refer to USB MIDI spec 1.0 https://www.usb.org/sites/default/files/midi10.pdf
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<(), usb_device::UsbError> {
        writer.interface(
            self.audio_subclass,
            USB_AUDIO_CLASS,
            USB_AUDIO_CONTROL_SUBCLASS,
            0x00, // no protocol
        )?;

        writer.write(CS_INTERFACE, &[
            HEADER_SUBTYPE,
            0x00,
            0x01, // Revision
            0x09,
            0x00, // SIZE of class specific descriptions
            0x01, // Number of streaming interfaces
            0x01, // MIDI Streaming interface 1 belongs to this AC interface
        ])?;

        // Streaming Standard
        writer.interface(
            self.midi_subclass,
            USB_AUDIO_CLASS,
            USB_MIDI_STREAMING_SUBCLASS,
            0,
        )?;

        // Streaming Extras
        writer.write(CS_INTERFACE, &[
            MS_HEADER_SUBTYPE,
            0x00,
            0x01, // Revision
            0x07 + MIDI_OUT_SIZE,
            0x00,
        ])?;

        // Jacks
        writer.write(CS_INTERFACE, &[MIDI_IN_JACK_SUBTYPE, EMBEDDED, 0x01, 0x00])?;

        writer.write(CS_INTERFACE, &[
            MIDI_OUT_JACK_SUBTYPE,
            EMBEDDED,
            0x01,
            0x01,
            0x01,
            0x01,
            0x00,
        ])?;

        writer.endpoint(&self.bulk_out)?;
        writer.write(CS_ENDPOINT, &[MS_GENERAL, 0x01, 0x01])?;

        writer.endpoint(&self.bulk_in)?;
        writer.write(CS_ENDPOINT, &[MS_GENERAL, 0x01, 0x01])?;
        Ok(())
    }
}
