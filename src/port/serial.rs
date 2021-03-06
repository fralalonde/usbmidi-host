//! MIDI using HAL Serial

use heapless::spsc::Queue;
use midi::{Packet, MidiError, CableNumber, PacketParser, is_channel_status};
use embedded_hal::serial;
use usb_host::USB_MIDI_PACKET_LEN;

pub struct UartMidi<UART> {
    pub uart: UART,
    pub tx_fifo: Queue<u8, 64>,
    cable_number: CableNumber,
    parser: PacketParser,
    last_status: Option<u8>,
}

impl<UART> UartMidi<UART>
    where UART: serial::Write<u8>,
{
    pub fn new(uart: UART, cable_number: CableNumber) -> Self {
        UartMidi {
            uart,
            tx_fifo: Queue::new(),
            cable_number,
            parser: PacketParser::default(),
            last_status: None,
        }
    }

    pub fn flush(&mut self) -> Result<(), MidiError> {
        'write_bytes:
        loop {
            if let Some(byte) = self.tx_fifo.dequeue() {
                self.uart.write(byte)?;
                continue 'write_bytes;
            }
            return Ok(());
        }
    }

    fn write_all(&mut self, payload: &[u8]) -> Result<(), MidiError> {
        for byte in payload {
            self.write_byte(*byte)?
        }
        Ok(())
    }

    fn write_byte(&mut self, byte: u8) -> Result<(), MidiError> {
        self.tx_fifo.enqueue(byte).map_err(|_| MidiError::BufferFull)?;
        Ok(())
    }
}

impl<UART> midi::Receive for UartMidi<UART> where
    UART: serial::Read<u8>,
{
    fn receive(&mut self) -> Result<Option<Packet>, MidiError> {
        // if self.uart.is_rxne() {
        let byte = self.uart.read()?;
        let packet = self.parser.advance(byte)?;
        if let Some(packet) = packet {
            return Ok(Some(packet.with_cable_num(self.cable_number)));
        }
        // }
        Ok(None)
    }
}

impl<UART> midi::Transmit for UartMidi<UART>
    where UART: serial::Write<u8>,
{
    // allow checking before trying to add packet
    fn is_tx_full(&self) -> bool {
        self.tx_fifo.len() + USB_MIDI_PACKET_LEN > self.tx_fifo.capacity()
    }

    //
    fn transmit(&mut self, packet: Packet) -> Result<(), MidiError> {
        let mut payload = packet.payload();
        // Apply MIDI "running status"
        if is_channel_status(payload[0]) {
            if let Some(last_status) = self.last_status {
                if payload[0] == last_status {
                    // same status as last time, chop out status byte
                    payload = &payload[1..];
                } else {
                    // take note of new status
                    self.last_status = Some(payload[0])
                }
            }
        } else {
            // non-repeatable status or no status (sysex)
            self.last_status = None
        }
        self.write_all(payload)?;

        self.flush()?;
        Ok(())
    }
}


