use core::fmt::{Formatter, Pointer};
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;

use core::sync::atomic::Ordering::Relaxed;
use core::task::{Context, Poll, Waker};

use atomic_polyfill::{AtomicU64};

use cortex_m::peripheral::{SYST};
use cortex_m::peripheral::syst::SystClkSource;

use embedded_time::clock::Error;
use embedded_time::fraction::Fraction;
use embedded_time::{Clock, Instant};
use embedded_time::duration::{Microseconds, Milliseconds, Nanoseconds};

use sync_thumbv6m::spin::SpinMutex;
use crate::pri_queue::PriorityQueue;

use cortex_m_rt::exception;
use sync_thumbv6m::alloc::Arc;
use crate::RuntimeError;

pub struct SysTickClock<const FREQ: u32> {
    systick: &'static mut SYST,
    past_cycles: AtomicU64,
}

impl<const FREQ: u32> core::fmt::Debug for SysTickClock<FREQ> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        self.systick.fmt(f)
    }
}

// const MAX_SYSTICK_CYCLES: u32 = 0x00ffffff;
const SYSTICK_CYCLES: u32 = 48_000_000;

// pub const SYSTICK_CYCLES: u32 = 48_000_000;

static mut CLOCK: MaybeUninit<SysTickClock<SYSTICK_CYCLES>> = MaybeUninit::uninit();

pub fn init() {
    unsafe { CLOCK = MaybeUninit::new(SysTickClock::new()) };
}

pub fn now() -> Instant<SysTickClock<SYSTICK_CYCLES>> {
    unsafe { CLOCK.assume_init_ref().now() }
}

pub fn later(cycles: u64) -> Instant<SysTickClock<SYSTICK_CYCLES>> {
    unsafe { CLOCK.assume_init_ref().later(cycles) }
}

pub fn now_millis() -> u64 {
    Milliseconds::try_from(now() - SysTickClock::zero()).unwrap().0
}

// dirty - should be passed as constructor parameter
#[allow(mutable_transmutes)]
fn syst() -> &'static mut SYST {
    unsafe { core::mem::transmute(&*SYST::ptr()) }
}

const MAX_RVR: u32 = 0x00FF_FFFF;

impl<const FREQ: u32> SysTickClock<FREQ> {
    fn new() -> Self {
        let syst = syst();
        syst.disable_interrupt();
        syst.disable_counter();
        syst.clear_current();

        syst.set_clock_source(SystClkSource::Core);
        syst.set_reload(MAX_RVR);

        syst.enable_counter();

        // actually enables the #[exception] SysTick (see below)
        syst.enable_interrupt();

        Self {
            systick: syst,
            past_cycles: AtomicU64::new(0),
        }
    }

    fn zero() -> Instant<Self> {
        Instant::new(0)
    }

    fn now(&self) -> Instant<Self> {
        Instant::new(self.cycles())
    }

    fn later(&self, period: u64) -> Instant<Self> {
        Instant::new(self.cycles() + period)
    }

    #[inline]
    fn cycles(&self) -> u64 {
        // systick cvr counts DOWN
        let elapsed_cycles = (MAX_RVR - self.systick.cvr.read());
        self.past_cycles.load(Relaxed) + elapsed_cycles as u64
    }

    #[inline]
    pub fn rollover(&self) {
        self.past_cycles.fetch_add(MAX_RVR as u64, Relaxed);
    }
}

#[exception]
fn SysTick() {
    unsafe { CLOCK.assume_init_ref().rollover() };
}

impl<const FREQ: u32> Clock for SysTickClock<FREQ> {
    type T = u64;

    const SCALING_FACTOR: Fraction = Fraction::new(1, FREQ);

    fn try_now(&self) -> Result<Instant<Self>, Error> {
        Ok(self.now())
    }
}

static SCHED: SpinMutex<PriorityQueue<Instant<SysTickClock<SYSTICK_CYCLES>>, Arc<dyn Fn() + 'static + Send + Sync>, 16>> = SpinMutex::new(PriorityQueue::new());

pub fn schedule_at<F: Fn() + 'static + Send + Sync>(when: Instant<SysTickClock<SYSTICK_CYCLES>>, what: F) {
    let mut sched = SCHED.lock();
    let f: Arc<dyn Fn() + 'static + Send + Sync> = Arc::new(what);
    if !sched.push(when, &f) {
        panic!("No scheduler slot left")
    }
}

pub fn run_scheduled() {
    let mut sched = SCHED.lock();
    while let Some(wake_fn) = sched.pop_due(now()) {
        wake_fn()
    }
}

pub fn delay_ms(duration: u32) -> AsyncDelay {
    let due_time = now() + Milliseconds(duration);
    delay_until(due_time)
}

pub fn delay_us(duration: u32) -> AsyncDelay {
    let due_time = now() + Microseconds(duration);
    delay_until(due_time)
}

pub fn delay_ns(duration: u32) -> AsyncDelay {
    let due_time = now() + Nanoseconds(duration);
    delay_until(due_time)
}

pub fn delay_cycles(duration: u64) -> AsyncDelay {
    let due_time = later(duration);
    delay_until(due_time)
}

pub fn delay_until(due_time: Instant<SysTickClock<SYSTICK_CYCLES>>) -> AsyncDelay {
    let waker: Arc<SpinMutex<Option<Waker>>> = Arc::new(SpinMutex::new(None));
    let sched_waker = waker.clone();
    schedule_at(due_time, move || {
        if let Some(waker) = sched_waker.lock().take() {
            waker.wake()
        }
    });
    AsyncDelay { waker, due_time }
}

pub struct AsyncDelay {
    waker: Arc<SpinMutex<Option<Waker>>>,
    due_time: Instant<SysTickClock<SYSTICK_CYCLES>>,
}

impl Future for AsyncDelay {
    type Output = Result<(), RuntimeError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let now = now();
        if self.due_time <= now {
            Poll::Ready(Ok(()))
        } else {
            let mut waker = self.waker.lock();
            *waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}