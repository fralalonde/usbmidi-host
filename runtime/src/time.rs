use core::fmt::{Formatter, Pointer};

use core::sync::atomic::Ordering::Relaxed;
use atomic_polyfill::AtomicU32;

use cortex_m::peripheral::{SYST};
use cortex_m::peripheral::syst::SystClkSource;

use cortex_m_rt::exception;
use fugit::{Duration, Instant};

use crate::{Local};

const SYSTICK_CYCLES: u32 = 48_000_000;

pub type SysInstant = Instant<u64, 1, SYSTICK_CYCLES>;
pub type SysDuration = Duration<u32, 1, SYSTICK_CYCLES>;

pub struct SysClock {
    syst: &'static mut SYST,
    past_cycles: AtomicU32,
}

impl core::fmt::Debug for SysClock {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.syst.fmt(f)
    }
}

static CLOCK: Local<SysClock> = Local::uninit("CLOCK");

pub fn init(syst: &'static mut SYST) {
    CLOCK.init_static(SysClock::new(syst));
}

pub fn now() -> SysInstant {
     CLOCK.now()
}

pub fn later(cycles: u64) -> SysInstant {
    CLOCK.later(cycles)
}

pub fn now_millis() -> u64 {
    (now() - SysClock::zero()).to_millis()
}

const MAX_RVR: u32 = 0x00FF_FFFF;

impl SysClock {
    fn new(syst: &'static mut SYST) -> Self {
        syst.disable_interrupt();
        syst.disable_counter();
        syst.clear_current();

        syst.set_clock_source(SystClkSource::Core);
        syst.set_reload(MAX_RVR);

        syst.enable_counter();

        // actually enables the #[exception] SysTick (see below)
        syst.enable_interrupt();

        Self {
            syst,
            past_cycles: AtomicU32::new(0),
        }
    }

    fn zero() -> SysInstant {
        SysInstant::from_ticks(0)
    }

    fn now(&self) -> SysInstant {
        SysInstant::from_ticks(self.cycles())
    }

    fn later(&self, period: u64) -> SysInstant {
        SysInstant::from_ticks(self.cycles() + period)
    }

    #[inline]
    fn cycles(&self) -> u64 {
        // systick cvr counts DOWN
        let elapsed_cycles = MAX_RVR - self.syst.cvr.read();
        self.past_cycles.load(Relaxed) as u64 + elapsed_cycles as u64
    }

    #[inline]
    pub fn rollover(&self) {
        self.past_cycles.fetch_add(MAX_RVR, Relaxed);
    }
}

#[exception]
fn SysTick() {
    CLOCK.rollover();
}

