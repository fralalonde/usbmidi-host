#![no_std]
#![no_main]

#[macro_use]
extern crate runtime;

extern crate embedded_midi as midi;

mod port;
mod usb_midi;

use trinket_m0 as bsp;

use bsp::clock::GenericClockController;
use bsp::entry;
use bsp::pac::{interrupt, CorePeripherals, Peripherals};

use cortex_m::peripheral::NVIC;

use trinket_m0::clock::{ClockGenId, ClockSource};

use core::mem;
use core::ops::DerefMut;

use atsamd_hal as hal;
use hal::pac;

use atsamd_usb_host::{Pins, SAMDHost};

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

use hal::delay::Delay;

use atsamd_hal::time::{Hertz};
use atsamd_hal::gpio::v2::*;
use atsamd_hal::sercom::UART0;

use crate::usb_midi::UsbMidiDriver;

use hal::sercom::*;
use atsamd_hal::gpio::{self, *};

use midi::{CableNumber, PacketList, Binding, Receive};


use atsamd_hal::gpio::PfD;

use atsamd_hal::rtc::Rtc;
use cortex_m::asm::delay;


use usb_host::{Address, AddressPool, Driver, HostEvent, SingleEp};

use heapless::Vec;
use runtime::{Local, Shared};
use crate::port::serial::SerialMidi;

static CORE: Local<CorePeripherals> = Local::uninit("CORE");

static UART_MIDI: Shared<SerialMidi<UART0<Sercom0Pad3<Pa7<PfD>>, Sercom0Pad2<Pa6<PfD>>, (), ()>>> = Shared::uninit("UART_MIDI");

static USB_HOST: Shared<SAMDHost> = Shared::uninit("USB_HOST");
static USB_MIDI_DRIVER: Shared<UsbMidiDriver> = Shared::uninit("USB_MIDI_DRIVER");
static USB_ADDR_POOL: Shared<AddressPool> = Shared::uninit("USB_ADDR_POOL");
static USB_MIDI_PORT: Shared<Option<SingleEp>> = Shared::uninit("USB_MIDI_PORT");

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
    // let mut delay = Delay::new(core.SYST, &mut clocks);


    let timer_clock = clocks
        .configure_gclk_divider_and_source(ClockGenId::GCLK4, 1, ClockSource::OSC32K, false)
        .unwrap();
    let tc45 = &clocks.tc4_tc5(&timer_clock).unwrap();

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
    let serial_midi = crate::port::serial::SerialMidi::new(serial, CableNumber::MIN);
    info!("Serial OK");

    let usb_pins = Pins::new(
        pins.usb_dm.into_floating_input(&mut pins.port),
        pins.usb_dp.into_floating_input(&mut pins.port),
        Some(pins.usb_sof.into_floating_input(&mut pins.port)),
        Some(pins.usb_host_enable.into_floating_input(&mut pins.port)),
    );

    let mut usb_host = SAMDHost::new(
        peripherals.USB,
        usb_pins,
        &mut pins.port,
        &mut clocks,
        &mut peripherals.PM,
        runtime::now_millis,
    );
    info!("USB Host OK");

    USB_MIDI_DRIVER.init_static(UsbMidiDriver::default());
    USB_ADDR_POOL.init_static(AddressPool::new());
    usb_host.reset_host();
    USB_HOST.init_static(usb_host);

    info!("Board Initialization Complete");

    unsafe {
        core.NVIC.set_priority(interrupt::USB, 3);
        NVIC::unmask(interrupt::USB);

        core.NVIC.set_priority(interrupt::SERCOM0, 3);
        NVIC::unmask(interrupt::SERCOM0);
    }

    // runtime::spawn(async {
    //     // let d = Drivers::new()
    //     loop {
    //         unsafe { USB_HOST.assume_init_mut().update(HostEvent::NoEvent, &mut USB_DRIVERS).await }
    //         runtime::delay_us(125).await;
    //     }
    // });

    loop {
        red_led.toggle();
        delay(12_000_000);
    }
}

fn midi_route(binding: Binding, packets: PacketList) {
    // let router: &mut route::Router = cx.resources.midi_router;
    // router.midi_route(cx.scheduled, packets, binding, cx.spawn).unwrap();
}

// #[interrupt]
// fn TC4() {
//     exec::spawn(usb_service(HostEvent::NoEvent));
//     unsafe { TC4::ptr().as_ref().unwrap().count16().intflag.modify(|_, w| w.ovf().set_bit()); }
// }

// #[interrupt]
// fn SERCOM0() {
//     debug!("serial irq");
//     trace!("IRQ SERCOM0");
//     if let Err(err) = cx.shared.serial_midi.lock(|m| m.flush()) {
//         error!("Serial flush failed {:?}", err);
//     }
//
//     while let Ok(Some(packet)) = cx.shared.serial_midi.lock(|m| m.receive()) {
//         midi_route::spawn(Src(UPSTREAM_SERIAL), PacketList::single(packet)).unwrap();
//     }
// }

#[interrupt]
fn USB() {
    NVIC::mask(interrupt::USB);
    let mut usb = USB_HOST.lock();
    let mut serial = UART_MIDI.lock();
    let mut drivers = USB_MIDI_DRIVER.lock();
    let mut addr_pool = USB_ADDR_POOL.lock();

    let usb_irq = usb.next_irq();
    if let Some(host_event) = usb.update(usb_irq, &mut addr_pool) {
        match host_event {
            HostEvent::Ready(device) => {
                info!("USB Host Ready {:?}", device)
                // TODO register device, call drivers for match
            }
            HostEvent::Reset => {
                info!("USB Host Reset")
                // TODO clear pool, call drivers for unregister
            }
            HostEvent::Tick => {
                // TODO call drivers for push reads
            }
        }
    }

    // TODO set / unset usb midi port on attach / detach

    // TODO if usb midi device connected  read any packet from port
    //  if let Some(port) = port {
    //      if let Err(e) = serial.write(&[byte]) {
    //          defmt::info!("USB write err {:?}", e);
    //      }
    //  }

    unsafe { NVIC::unmask(interrupt::USB) }
}

#[interrupt]
fn SERCOM0() {
    NVIC::mask(interrupt::SERCOM0);

    let mut usb = USB_HOST.lock();
    let mut serial = UART_MIDI.lock();
    let mut usb_midi = USB_MIDI_DRIVER.lock();
    let port = USB_MIDI_PORT.lock();

    // if let Ok(byte) = serial.receive() {
    //     if let Some(port) = port {
    //         usb_midi,transmit(port, &[byte])
    //     }
    // }

    unsafe { NVIC::unmask(interrupt::SERCOM0) };
}


