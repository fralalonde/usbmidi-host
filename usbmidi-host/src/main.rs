#![no_std]
#![no_main]

#[macro_use]
extern crate runtime;

extern crate embedded_midi as midi;

extern crate embedded_usb_host as usb_host;

mod port;

use trinket_m0 as bsp;

use bsp::clock::GenericClockController;
use bsp::entry;
use bsp::pac::{interrupt, CorePeripherals, Peripherals};

use cortex_m::peripheral::NVIC;

use trinket_m0::clock::{ClockGenId, ClockSource};

use crate::midi::MidiPorts;
use core::ops::{ DerefMut};

use atsamd_hal as hal;


use hal::sercom::{
    v2::{
        uart::{self, BaudMode, Oversampling},
        Sercom0,
        Sercom2,
    },
    I2CMaster3,
    I2CMaster2,
    I2CMaster1,
    I2CMaster0,
};

use atsamd_hal::time::{Hertz};
use atsamd_hal::gpio::v2::*;
use atsamd_hal::sercom::UART0;

use embedded_usb_host::driver::UsbMidiDriver;

use hal::sercom::*;
use atsamd_hal::gpio::{self, *};

use midi::{CableNumber, PacketList, Binding, MidiRegistry, PortId, Transmit, Packet, MidiError, PortDirection};


use atsamd_hal::gpio::PfD;

use atsamd_hal::rtc::Rtc;
use cortex_m::asm::delay;


use usb_host::{ AddressPool, atsamd, Driver, HostEvent, SingleEp, UsbStack};

use runtime::{Local, Shared};
use crate::port::serial::UartMidi;

static CORE: Local<CorePeripherals> = Local::uninit("CORE");

static UART_MIDI: Shared<UartMidi<UART0<Sercom0Pad3<Pa7<PfD>>, Sercom0Pad2<Pa6<PfD>>, (), ()>>> = Shared::uninit("UART_MIDI");
static MIDI_PORTS: Shared<MidiRegistry<2>> = Shared::uninit("MIDI_PORTS");

static USB_MIDI_DRIVER: Local<UsbMidiDriver> = Local::uninit("USB_MIDI_DRIVER");

static USB_STACK: Shared<UsbStack<atsamd::HostController>> = Shared::uninit("USB_STACK");

const RXC: u8 = 0x04;

#[entry]
fn main() -> ! {
    let mut peripherals = Peripherals::take().unwrap();
    let mut core = CORE.init_static(CorePeripherals::take().unwrap());
    runtime::init(&mut core.SYST);

    // internal 32khz required for USB to complete swrst
    let mut clocks = GenericClockController::with_internal_32kosc(
        peripherals.GCLK,
        &mut peripherals.PM,
        &mut peripherals.SYSCTRL,
        &mut peripherals.NVMCTRL,
    );

    // let _gclk = clocks.gclk0();
    // let rtc_clock_src = clocks
    //     .configure_gclk_divider_and_source(ClockGenId::GCLK2, 1, ClockSource::OSC32K, false)
    //     .unwrap();
    // clocks.configure_standby(ClockGenId::GCLK2, true);
    // let rtc_clock = clocks.rtc(&rtc_clock_src).unwrap();
    // let rtc = Rtc::count32_mode(peripherals.RTC, rtc_clock.freq(), &mut peripherals.PM);

    let mut pins = bsp::Pins::new(peripherals.PORT);
    let mut red_led = pins.d13.into_open_drain_output(&mut pins.port);

    let timer_clock = clocks
        .configure_gclk_divider_and_source(ClockGenId::GCLK4, 1, ClockSource::OSC32K, false)
        .unwrap();
    // let tc45 = &clocks.tc4_tc5(&timer_clock).unwrap();

    // let mut tc4 = TimerCounter::tc4_(tc45, peripherals.TC4, &mut peripherals.PM);
    // tc4.start(800.hz());
    // tc4.enable_interrupt();

    let mut serial: UART0<Sercom0Pad3<Pa7<PfD>>, Sercom0Pad2<Pa6<PfD>>, (), ()> = bsp::uart(
        &mut clocks,
        Hertz(921600),
        peripherals.SERCOM0,
        &mut peripherals.PM,
        pins.d3.into_floating_input(&mut pins.port),
        pins.d4.into_floating_input(&mut pins.port),
        &mut pins.port,
    );
    serial.intenset(|r| unsafe { r.bits(RXC); });
    let serial_midi = crate::port::serial::UartMidi::new(serial, CableNumber::MIN);
    info!("Serial OK");

    let usb_pins = atsamd::HostPins::new(
        pins.usb_dm.into_floating_input(&mut pins.port),
        pins.usb_dp.into_floating_input(&mut pins.port),
        Some(pins.usb_sof.into_floating_input(&mut pins.port)),
        Some(pins.usb_host_enable.into_floating_input(&mut pins.port)),
    );

    let mut usb_host = embedded_usb_host::atsamd::HostController::new(
        peripherals.USB,
        usb_pins,
        &mut pins.port,
        &mut clocks,
        &mut peripherals.PM,
        runtime::now_millis,
    );
    info!("USB Host OK");

    let mut driver = UsbMidiDriver::new(with_midi);
    let usb_driver = USB_MIDI_DRIVER.init_static(driver);
    usb_host.reset_host();

    USB_STACK.init_static(UsbStack::new(usb_host, usb_driver));

    info!("Board Initialization Complete");

    unsafe {
        core.NVIC.set_priority(interrupt::USB, 3);
        NVIC::unmask(interrupt::USB);

        core.NVIC.set_priority(interrupt::SERCOM0, 3);
        NVIC::unmask(interrupt::SERCOM0);
    }

    loop {
        red_led.toggle();
        delay(12_000_000);
        info!("time is {}", runtime::now_millis())
    }
}

fn with_midi(fun: &mut dyn FnMut(&mut (dyn MidiPorts + Send + Sync))) {
    let mut mlock = MIDI_PORTS.lock();
    // let midi = MIDI_PORTS.lock().deref_mut();
    fun(mlock.deref_mut())
}

fn midi_route(binding: Binding, packets: PacketList) {
    // let router: &mut route::Router = cx.resources.midi_router;
    // router.midi_route(cx.scheduled, packets, binding, cx.spawn).unwrap();
}

#[interrupt]
fn USB() {
    NVIC::mask(interrupt::USB);
    let mut usb = USB_STACK.lock();
    // process any changes or data
    usb.handle_irq();

    let mut midi = MIDI_PORTS.lock();

    // copy MIDI packets from first found USB port to UART
    for handle in midi.list_ports().iter().next() {
        if let Ok(info) = midi.info(handle) {
            if matches!(info.port_id, PortId::Usb(_)) && matches!(info.direction, PortDirection::In) {
                let mut serial = UART_MIDI.lock();
                loop {
                    // dont read from USB if serial buffer is full
                    // if serial.is_tx_full() { break; }
                    match midi.read(handle) {
                        Ok(Some(packet)) => {
                            info!("!!! got usb midi packet");
                            if let Err(err) = serial.transmit(packet) {
                                warn!("Serial MIDI error")
                            }
                        }
                        Err(err) => {
                            warn!("Failed to read from port: {}", info);
                            break;
                        }

                        Ok(None) => break
                    }
                }
            }
        }
    }

    unsafe { NVIC::unmask(interrupt::USB) }
}

#[interrupt]
fn SERCOM0() {
    NVIC::mask(interrupt::SERCOM0);

    let mut usb = USB_STACK.lock();
    let mut serial = UART_MIDI.lock();

    // if let Ok(byte) = serial.receive() {
    //     if let Some(port) = port {
    //         usb_midi, transmit(port, &[byte])
    //     }
    // }

    unsafe { NVIC::unmask(interrupt::SERCOM0) };
}


