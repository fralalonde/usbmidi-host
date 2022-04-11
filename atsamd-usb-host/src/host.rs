use async_trait::async_trait;
use alloc::boxed::Box;

use usb_host::{
    DescriptorType, DeviceDescriptor, Direction, DriverError, Endpoint, RequestCode,
    RequestDirection, RequestKind, RequestRecipient, RequestType, TransferError, TransferType,
    USBHost, WValue,
};

use atsamd_hal::{
    calibration::{usb_transn_cal, usb_transp_cal, usb_trim_cal},
    clock::{ClockGenId, ClockSource, GenericClockController},
    gpio::{self, Floating, Input, OpenDrain, Output},
    target_device::{PM, USB},
};
use embedded_hal::digital::v2::OutputPin;

#[derive(Debug, enumset::EnumSetType)]
pub enum HostEvent {
    NoEvent,
    Detached,
    Attached,
    RamAccess,
    UpstreamResume,
    DownResume,
    WakeUp,
    Reset,
    HostStartOfFrame,
}

const NAK_LIMIT: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, defmt::Format)]
enum DetachedState {
    Initialize,
    WaitForDevice,
}

#[derive(Clone, Copy, PartialEq, Debug, defmt::Format)]
enum AttachedState {
    ResetBus,
    WaitResetComplete,
    /// Instant at which waiting started, in milliseconds, wrapping
    WaitSOF(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, defmt::Format)]
enum SteadyState {
    Configuring,
    Running,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, defmt::Format)]
enum TaskState {
    Detached(DetachedState),
    Attached(AttachedState),
    Steady(SteadyState),
}

use core::mem::{self};
use usb_host::address::AddressPool;
use usb_host::device::Device;
use usb_host::parser::DescriptorParser;
use crate::pipe::table::PipeTable;

/// The maximum size configuration descriptor we can handle.
const CONFIG_BUFFER_LEN: usize = 255;

pub struct SAMDHost {
    usb: USB,
    task_state: TaskState,

    // Need chunk of RAM for USB pipes, which gets used with DESCADD
    // register.
    pipe_table: PipeTable,

    addr_pool: AddressPool,

    _dm_pad: gpio::Pa24<gpio::PfG>,
    _dp_pad: gpio::Pa25<gpio::PfG>,
    _sof_pad: Option<gpio::Pa23<gpio::PfG>>,
    host_enable_pin: Option<gpio::Pa28<Output<OpenDrain>>>,
    millis: fn() -> u64,
}

// FIXME why isn't atsamd21e::USB Sync ?
unsafe impl Sync for SAMDHost {}

pub struct Pins {
    dm_pin: gpio::Pa24<Input<Floating>>,
    dp_pin: gpio::Pa25<Input<Floating>>,
    sof_pin: Option<gpio::Pa23<Input<Floating>>>,
    host_enable_pin: Option<gpio::Pa28<Input<Floating>>>,
}

impl Pins {
    pub fn new(
        dm_pin: gpio::Pa24<Input<Floating>>,
        dp_pin: gpio::Pa25<Input<Floating>>,
        sof_pin: Option<gpio::Pa23<Input<Floating>>>,
        host_enable_pin: Option<gpio::Pa28<Input<Floating>>>,
    ) -> Self {
        Self {
            dm_pin,
            dp_pin,
            sof_pin,
            host_enable_pin,
        }
    }
}

impl SAMDHost {
    pub fn new(
        usb: USB,
        pins: Pins,
        port: &mut gpio::Port,
        clocks: &mut GenericClockController,
        power: &mut PM,
        millis: fn() -> u64,
    ) -> Self {
        power.apbbmask.modify(|_, w| w.usb_().set_bit());

        clocks.configure_gclk_divider_and_source(ClockGenId::GCLK6, 1, ClockSource::DFLL48M, false);
        let gclk6 = clocks.get_gclk(ClockGenId::GCLK6).expect("Could not get clock 6");
        clocks.usb(&gclk6);

        SAMDHost {
            usb,
            task_state: TaskState::Detached(DetachedState::Initialize),
            pipe_table: PipeTable::new(),
            addr_pool: AddressPool::new(),

            _dm_pad: pins.dm_pin.into_function_g(port),
            _dp_pad: pins.dp_pin.into_function_g(port),
            _sof_pad: pins.sof_pin.map(|p| p.into_function_g(port)),
            host_enable_pin: pins.host_enable_pin.map(|p| p.into_open_drain_output(port)),
            millis,
        }
    }

    /// Low-Level USB Host Interrupt service method
    /// Any Event returned by should be sent to process_event()
    /// then fsm_tick() should be called for each event or once if no event at all
    pub fn irq_next_event(&self) -> Option<HostEvent> {
        let flags = self.usb.host().intflag.read();

        if flags.ddisc().bit_is_set() {
            self.usb.host().intflag.write(|w| w.ddisc().set_bit());
            Some(HostEvent::Detached)
        } else if flags.dconn().bit_is_set() {
            self.usb.host().intflag.write(|w| w.dconn().set_bit());
            Some(HostEvent::Attached)
        } else if flags.ramacer().bit_is_set() {
            self.usb.host().intflag.write(|w| w.ramacer().set_bit());
            Some(HostEvent::RamAccess)
        } else if flags.uprsm().bit_is_set() {
            self.usb.host().intflag.write(|w| w.uprsm().set_bit());
            Some(HostEvent::UpstreamResume)
        } else if flags.dnrsm().bit_is_set() {
            self.usb.host().intflag.write(|w| w.dnrsm().set_bit());
            Some(HostEvent::DownResume)
        } else if flags.wakeup().bit_is_set() {
            self.usb.host().intflag.write(|w| w.wakeup().set_bit());
            Some(HostEvent::WakeUp)
        } else if flags.rst().bit_is_set() {
            // self.usb.host().intflag.write(|w| w.rst().set_bit());
            Some(HostEvent::Reset)
        } else if flags.hsof().bit_is_set() {
            // self.usb.host().intflag.write(|w| w.hsof().set_bit());
            Some(HostEvent::HostStartOfFrame)
        } else {
            None
        }
    }

    /// Events from interrupts are processed here to avoid locking IRQ routine for too long
    pub async fn update(&mut self, event: Option<HostEvent>, drivers: &mut [&'static mut (dyn usb_host::Driver + Send + Sync)]) {
        // trace!("USB Event [{:?}]", event);
        let prev = self.task_state;

        if let Some(event) = event {
            self.task_state = match (event, self.task_state) {
                // continue detaching
                (HostEvent::Detached, TaskState::Detached(_)) => self.task_state,
                // initiate detach
                (HostEvent::Detached, _) => TaskState::Detached(DetachedState::Initialize),
                // attaching while detach in progress? OwO -> reset bus
                (HostEvent::Attached, TaskState::Detached(_)) => TaskState::Attached(AttachedState::ResetBus),
                // event does not change state
                _ => self.task_state
            }
        }

        self.task_state = match self.task_state {
            TaskState::Detached(DetachedState::Initialize) => {
                self.reset_host();
                // TODO Free resources
                TaskState::Detached(DetachedState::WaitForDevice)
            }

            TaskState::Attached(AttachedState::ResetBus) => {
                // self.reset_bus().await.unwrap();
                self.usb.host().ctrlb.modify(|_, w| w.busreset().set_bit());
                TaskState::Attached(AttachedState::WaitResetComplete)
            }
            TaskState::Attached(AttachedState::WaitResetComplete) if self.usb.host().intflag.read().rst().bit_is_set() => {
                // Seems unnecessary, since SOFE will be set immediately after reset according to §32.6.3.3.
                self.usb.host().ctrlb.modify(|_, w| w.sofe().set_bit());
                // USB spec requires 20ms of SOF after bus reset.
                TaskState::Attached(AttachedState::WaitSOF((self.millis)() + 20))
            }
            TaskState::Attached(AttachedState::WaitSOF(until)) if self.usb.host().intflag.read().hsof().bit_is_set() => {
                self.usb.host().intflag.write(|w| w.hsof().set_bit());
                if (self.millis)() >= until {
                    TaskState::Steady(SteadyState::Configuring)
                } else {
                    self.task_state
                }
            }
            TaskState::Steady(SteadyState::Configuring) => {
                match self.configure_dev(drivers).await {
                    Ok(_) => TaskState::Steady(SteadyState::Running),
                    Err(e) => {
                        warn!("USB Enumeration Error: {:?}", e);
                        TaskState::Steady(SteadyState::Error)
                    }
                }
            }
            TaskState::Steady(SteadyState::Running) => {
                for driver in drivers {
                    if let Err(e) = driver.tick(self).await {
                        warn!("USB Driver Error: {:?}",  e);
                        // if let DriverError::Permanent(addr, _) = e {
                        //     driver.remove_device(addr);
                        //     self.addr_pool.put_back(addr);
                        // }
                    }
                }
                self.task_state
            }
            state => state
        };
        if prev != self.task_state {
            debug!("USB State [{:?}] -> [{:?}]", prev, self.task_state);
        }
    }

    pub fn reset_host(&mut self) {
        self.usb.host().ctrla.write(|w| w.swrst().set_bit());
        while self.usb.host().syncbusy.read().swrst().bit_is_set() {}

        self.usb.host().ctrla.modify(|_, w| w.mode().host());

        unsafe {
            self.usb.host().padcal.write(|w| {
                w.transn().bits(usb_transn_cal());
                w.transp().bits(usb_transp_cal());
                w.trim().bits(usb_trim_cal())
            });
        }

        self.usb.host().ctrlb.modify(|_, w| w.spdconf().normal());
        self.usb.host().ctrla.modify(|_, w| w.runstdby().set_bit());

        unsafe { self.usb.host().descadd.write(|w| w.bits(&self.pipe_table as *const _ as u32)); }

        if let Some(host_enable_pin) = &mut self.host_enable_pin {
            host_enable_pin.set_high().expect("USB Reset [host enable pin]");
        }

        self.usb.host().intenset.write(|w| {
            w.dconn().set_bit();
            w.ddisc().set_bit();
            w.wakeup().set_bit();
            // w.uprsm().set_bit();
            // w.dnrsm().set_bit();
            // w.rst().set_bit();
            // w.hsof().set_bit();
            w
        });

        self.usb.host().ctrla.modify(|_, w| w.enable().set_bit());
        while self.usb.host().syncbusy.read().enable().bit_is_set() {}
        self.usb.host().ctrlb.modify(|_, w| w.vbusok().set_bit());
        debug!("USB Host Reset");
    }

    async fn reset_bus(&self) -> Result<(), TransferError> {
        self.usb.host().ctrlb.modify(|_, w| w.busreset().set_bit());
        // runtime::delay_ms(20).await?;
        debug!("USB Device Reset");
        Ok(())
    }

    async fn configure_dev(&mut self, drivers: &mut [&'static mut (dyn usb_host::Driver + Send + Sync)]) -> Result<(), TransferError> {
        debug!("USB Configuring Device");
        let max_bus_packet_size: u16 = match self.usb.host().status.read().speed().bits() {
            0x0 => 64,
            _ => 8,
        };
        let (mut device, dev_desc) = self.config_dev_loop(max_bus_packet_size).await?;
        runtime::delay_ms(10).await?;

        // self.usb.host().intenset.write(|w| {
        //     w.hsof().set_bit()
        // });

        debug!("USB Describing Device");
        let mut cfg_buf = [0; CONFIG_BUFFER_LEN];
        let len = device.get_configuration_descriptors(self, 0, &mut cfg_buf).await?;
        let mut parser = usb_host::parser::DescriptorParser::new(&cfg_buf[0..len]);

        for d in drivers.iter_mut() {
            match d.connected(self, &mut device, &dev_desc, &mut parser).await {
                Ok(true) => break,
                Err(e) => error!("Driver error on connect {:?}", e),
                _ => {}
            };
            parser.rewind()
        }
        // TODO store device state?
        Ok(())
    }

    async fn config_dev_loop(&mut self, max_bus_packet_size: u16) -> Result<(Device, DeviceDescriptor), TransferError> {
        let mut retries = 0;
        loop {
            match self.config_dev_inner(max_bus_packet_size).await {
                Ok(dev) => return Ok(dev),
                Err(err) => {
                    warn!("USB Device enumeration error {:?}", err);
                    retries += 1;
                    if retries > 2 {
                        return Err(TransferError::EnumerationFailed);
                    }
                }
            }
        }
    }

    async fn config_dev_inner(&mut self, max_bus_packet_size: u16) -> Result<(Device, DeviceDescriptor), TransferError> {
        let mut device = Device::new(max_bus_packet_size);
        let dev_desc = device.get_device_descriptor(self).await?;
        debug!("USB Device {:?}", dev_desc);

        let dev_addr = self.addr_pool.take_next().ok_or(TransferError::Permanent("Out of USB addr"))?;
        device.set_address(self, dev_addr).await?;
        debug!("USB Device assigned address {:?}", device.get_address());

        Ok((device, dev_desc))
    }
}

#[async_trait]
impl USBHost for SAMDHost {
    async fn get_host_id(&self) -> u8 {
        // TODO incremental host ids
        0
    }

    async fn control_transfer(&mut self, ep: &dyn Endpoint, req_type: RequestType, req_code: RequestCode,
                              w_value: WValue, w_index: u16, buf: Option<&mut [u8]>) -> Result<usize, TransferError> {
        let mut pipe = self.pipe_table.pipe_for(self.usb.host_mut(), ep, self.millis);
        let len = pipe.control_transfer(ep, req_type, req_code, w_value, w_index, buf).await?;
        Ok(len)
    }

    async fn in_transfer(&mut self, ep: &dyn Endpoint, buf: &mut [u8]) -> Result<usize, TransferError> {
        if ep.direction() != Direction::In {
            return Err(TransferError::Permanent("Endpoint transfer direction mismatch"));
        }
        let mut pipe = self.pipe_table.pipe_for(self.usb.host_mut(), ep, self.millis);
        let len = pipe.in_transfer(ep, buf).await?;
        Ok(len)
    }

    async fn out_transfer(&mut self, ep: &dyn Endpoint, buf: &[u8]) -> Result<usize, TransferError> {
        if ep.direction() != Direction::Out {
            return Err(TransferError::Permanent("Endpoint transfer direction mismatch"));
        }
        let mut pipe = self.pipe_table.pipe_for(self.usb.host_mut(), ep, self.millis);
        let len = pipe.out_transfer(ep, buf).await?;
        Ok(len)
    }
}
