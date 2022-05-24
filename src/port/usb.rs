use heapless::Deque;
use midi::{Packet, MidiError, PacketList, Transmit, Receive};
use usb_host::{Address, SingleEp};

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

const PACKET_LEN: usize = 4;
const TX_FIFO_SIZE: usize = USB_TX_BUFFER_SIZE as usize;
const RX_FIFO_SIZE: usize = USB_RX_BUFFER_SIZE as usize + PACKET_LEN;

impl Transmit for UsbMidi {
    fn transmit(&mut self, packets: PacketList) -> Result<(), MidiError> {
        for packet in packets.iter() {
            self.midi_class.tx_push(packet.bytes());
        }
        self.midi_class.tx_flush();
        Ok(())
    }
}

impl Receive for UsbMidi {
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
pub struct UsbMidi {
    bulk_out: SingleEp,
    bulk_in: SingleEp,
    tx_fifo: Deque<u8, TX_FIFO_SIZE>,
    rx_fifo: Deque<u8, TX_FIFO_SIZE>,
}

impl UsbMidi {
    /// Creates a new MidiClass with the provided UsbBus
    pub fn new() -> UsbMidi {
        UsbMidi {
            bulk_out: usb_alloc.bulk(USB_TX_BUFFER_SIZE),
            bulk_in: usb_alloc.bulk(USB_RX_BUFFER_SIZE),

            tx_fifo: [0; TX_FIFO_SIZE],
            tx_len: 0,

            rx_fifo: [0; RX_FIFO_SIZE],
            rx_start: 0,
            rx_end: 0,
        }
    }

    /// Callback after USB flush (send) completed
    /// Check for packets that were enqueued while devices was busy (UsbErr::WouldBlock)
    /// If any packets are pending re-flush queue immediately
    /// This callback may chain-trigger under high output load (big sysex, etc.) - this is good
    fn endpoint_in_complete(&mut self, addr: Address) {
        if addr == self.bulk_in.address() {
            if self.tx_len > 0 {
                // send pending bytes in tx_buf
                self.tx_flush();
            }
        }
    }

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
        for b in payload {
            if !self.tx_fifo.push_front(*b).is_ok() {
                return false
            }
        }
        true
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

    /// Dequeue a packet from RX FIFO (if any)
    fn rx_pop(&mut self) -> Option<Packet> {
        if self.rx_size() >= PACKET_LEN {
            let raw = self.rx_fifo.as_chunks().0[0];
            self.rx_start += PACKET_LEN;
            return Some(raw);
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
