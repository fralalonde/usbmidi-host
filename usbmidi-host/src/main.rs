#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
extern crate runtime;

extern crate embedded_midi as midi;

use buddy_alloc::{BuddyAllocParam, FastAllocParam, NonThreadsafeAlloc};
use runtime::cxalloc::CortexMSafeAlloc;

const FAST_HEAP_SIZE: usize = 8 * 1024;
const HEAP_SIZE: usize = 8 * 1024;
const LEAF_SIZE: usize = 16;

pub static mut FAST_HEAP: [u8; FAST_HEAP_SIZE] = [0u8; FAST_HEAP_SIZE];
pub static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];

#[cfg_attr(not(test), global_allocator)]
static ALLOC: CortexMSafeAlloc = unsafe {
    let fast_param = FastAllocParam::new(FAST_HEAP.as_ptr(), FAST_HEAP_SIZE);
    let buddy_param = BuddyAllocParam::new(HEAP.as_ptr(), HEAP_SIZE, LEAF_SIZE);
    CortexMSafeAlloc(NonThreadsafeAlloc::new(fast_param, buddy_param))
};

mod port;
mod usb_midi;

use trinket_m0 as bsp;

use bsp::clock::GenericClockController;
use bsp::entry;
use bsp::pac::{interrupt, CorePeripherals, Peripherals};

use cortex_m::peripheral::NVIC;

use trinket_m0::clock::{ClockGenId, ClockSource};
use trinket_m0::time::U32Ext;

use core::mem;

use atsamd_hal as hal;
use hal::pac;

use atsamd_usb_host::{HostEvent, Pins, SAMDHost};

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

// use crate::usb_midi::MidiDriver;

use hal::sercom::*;
use atsamd_hal::gpio::{self, *};

use midi::{CableNumber, Interface, PacketList, Binding, Receive};
use crate::port::serial::SerialMidi;

use atsamd_hal::gpio::PfD;
use midi::Binding::Src;
use core::mem::{MaybeUninit};
use atsamd_hal::rtc::Rtc;

use sync_thumbv6m::alloc::Arc;
use usb_host::{Driver};
use embedded_time::Clock;
use heapless::Vec;

use sync_thumbv6m::spin::SpinMutex;
use crate::usb_midi::MidiDriver;

const UPSTREAM_SERIAL: Interface = Interface::Serial(0);

static mut MIDI_DRIVER: Option<MidiDriver> = None;

static mut USB_HOST: MaybeUninit<SAMDHost> = mem::MaybeUninit::uninit();

static mut USB_DRIVERS: Vec<&'static mut (dyn Driver + Send + Sync), 4> = heapless::Vec::new();

const RXC: u8 = 0x04;

#[entry]
fn main() -> ! {
    let mut peripherals = Peripherals::take().unwrap();
    let mut core = CorePeripherals::take().unwrap();

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

    runtime::init();

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

    let mut midi_driver = unsafe {
        MIDI_DRIVER = Some(MidiDriver::default());
        MIDI_DRIVER.as_mut().unwrap()
    };

    unsafe {
        USB_DRIVERS.push(midi_driver);
        usb_host.reset_host();
        USB_HOST = MaybeUninit::new(usb_host);
    };

    info!("Board Initialization Complete");

    unsafe {
        core.NVIC.set_priority(interrupt::USB, 3);
        NVIC::unmask(interrupt::USB);

        // core.NVIC.set_priority(interrupt::SERCOM0, 3);
        // NVIC::unmask(interrupt::SERCOM0);
    }

    // runtime::spawn(async {
    //     // let d = Drivers::new()
    //     loop {
    //         unsafe { USB_HOST.assume_init_mut().update(HostEvent::NoEvent, &mut USB_DRIVERS).await }
    //         runtime::delay_us(125).await;
    //     }
    // });

    runtime::spawn(async move {
        let mut prev = runtime::now_millis();
        loop {
            runtime::delay_ms(20000).await;
            let now = runtime::now_millis();
            error!("timer bongo {} {} {}ms", now, prev, now - prev);
            prev = now;
        }
    });

    runtime::spawn(async move {
        loop {
            red_led.toggle();
            runtime::delay_ms(500).await;
        }
    });

    loop {
        // // wake up
        runtime::run_scheduled();
        // // do things
        runtime::process_queue();
        // breathe?
        // cortex_m::asm::delay(400);
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
fn SERCOM0() {
    // debug!("serial irq");
    // trace!("IRQ SERCOM0");
    // if let Err(err) = cx.shared.serial_midi.lock(|m| m.flush()) {
    //     error!("Serial flush failed {:?}", err);
    // }
    //
    // while let Ok(Some(packet)) = cx.shared.serial_midi.lock(|m| m.receive()) {
    //     midi_route::spawn(Src(UPSTREAM_SERIAL), PacketList::single(packet)).unwrap();
    // }
}

#[interrupt]
unsafe fn USB() {
    let host_event = USB_HOST.assume_init_ref().irq_next_event();
    runtime::spawn(
        async move {
            // if let Some(host_event) = host_event {
                unsafe { USB_HOST.assume_init_mut().update(host_event, &mut USB_DRIVERS).await }
            // }
        })
}

