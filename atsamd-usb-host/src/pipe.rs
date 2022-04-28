#[allow(unused)]
pub mod addr;
#[allow(unused)]
pub mod ctrl_pipe;
#[allow(unused)]
pub mod ext_reg;
#[allow(unused)]
pub mod pck_size;
#[allow(unused)]
pub mod status_bk;
#[allow(unused)]
pub mod status_pipe;

pub mod table;
pub mod regs;

use async_trait::async_trait;

use addr::Addr;
use ctrl_pipe::CtrlPipe;
use ext_reg::ExtReg;
use pck_size::PckSize;
use status_bk::StatusBk;
use status_pipe::StatusPipe;

use usb_host::{
    Endpoint, RequestCode, RequestDirection, RequestType, SetupPacket, TransferType, WValue,
};

use atsamd_hal::target_device::usb::{
    self,
    host::{BINTERVAL, PCFG, PINTFLAG, PSTATUS, PSTATUSCLR, PSTATUSSET},
};
use core::convert::TryInto;
use core::mem;
use core::option::Option::None;

use crate::error::PipeErr;
use crate::pipe::regs::PipeRegs;

// Maximum time to wait for a control request with data to finish. cf
// §9.2.6.1 of USB 2.0.
const USB_TIMEOUT_MS: u64 = 5 * 1024; // 5 Seconds

// samd21 only supports 8 pipes.
const MAX_PIPES: usize = 8;

// How many times to retry a transaction that has transient errors.
const NAK_LIMIT: usize = 15;

// TODO: hide regs/desc fields. Needed right now for init_pipe0.

pub(crate) struct Pipe<'a, 'b> {
    pub(crate) num: usize,
    pub(crate) regs: PipeRegs<'b>,
    pub(crate) desc: &'a mut PipeDesc,
    pub(crate) millis: fn() -> u64,
}

impl Pipe<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn control_transfer(
        &mut self, ep: &dyn Endpoint, bm_request_type: RequestType,
        b_request: RequestCode, w_value: WValue, w_index: u16, buf: Option<&mut [u8]>,
    ) -> Result<usize, PipeErr>
    {
        debug!("USB Pipe[{}] CTRL Transfer [{:?}]", self.num, bm_request_type);

        let w_length = buf.as_ref().map_or(0, |b| b.len() as u16);
        let mut setup_packet = SetupPacket { bm_request_type, b_request, w_value, w_index, w_length };
        let buf_addr = &mut setup_packet as *mut SetupPacket as u32;
        self.desc.bank0.addr.write(|w| unsafe { w.addr().bits(buf_addr) });
        self.bank0_size(mem::size_of::<SetupPacket>() as u16);
        self.sync_tx(PToken::Setup, None).await?;

        // Data stage
        let mut transfer_len = 0;
        if let Some(buffer) = buf {
            // TODO: data stage, has up to 5,000ms (in 500ms per-packet chunks) to complete.
            // cf §9.2.6.4 of USB 2.0.
            match bm_request_type.direction() {
                RequestDirection::DeviceToHost => transfer_len = self.in_transfer(ep, buffer).await?,
                RequestDirection::HostToDevice => transfer_len = self.out_transfer(ep, buffer).await?,
            }
        }

        // Status stage has up to 50ms to complete
        // FIXME host is supposed to handle data toggle but we get err if we don't set toggle bit for status stage
        self.regs.statusset.write(|w| w.dtgl().set_bit());

        self.bank0_size(0);

        let token = match bm_request_type.direction() {
            RequestDirection::DeviceToHost => PToken::Out,
            RequestDirection::HostToDevice => PToken::In,
        };

        self.sync_tx(token, None).await?;
        Ok(transfer_len)
    }

    fn bank0_size(&mut self, len: u16) {
        self.desc.bank0.pcksize.modify(|_, w| {
            unsafe { w.byte_count().bits(len) };
            unsafe { w.multi_packet_size().bits(0) }
        });
    }

    pub(crate) async fn in_transfer(&mut self, endpoint: &dyn Endpoint, read_buf: &mut [u8]) -> Result<usize, PipeErr> {
        trace!("USB Pipe[{}] IN up to {} bytes, ep {:?}", self.num, read_buf.len(), endpoint.endpoint_address());
        self.bank0_size(read_buf.len() as u16);

        // Read until we get a short packet or the buffer is full
        let mut total_bytes = 0;
        loop {
            // Move the buffer pointer forward as we get data.
            self.desc.bank0.addr.write(|bank0| unsafe { bank0.addr().bits(read_buf.as_mut_ptr() as u32 + total_bytes as u32) });
            self.regs.statusclr.write(|w| w.bk0rdy().set_bit());

            self.sync_tx(PToken::In, None).await?;
            let byte_count = self.desc.bank0.pcksize.read().byte_count().bits();
            total_bytes += byte_count as usize;

            // short read => final chunk
            if byte_count < endpoint.max_packet_size() { break; }
            if total_bytes >= read_buf.len() { break; }
        }
        // TODO return subslice of buffer for safe short packet
        trace!("USB Pipe[{}] received {} bytes", self.num, total_bytes);
        Ok(total_bytes)
    }

    pub(crate) async fn out_transfer(&mut self, ep: &dyn Endpoint, buf: &[u8]) -> Result<usize, PipeErr> {
        trace!("USB Pipe[{}] OUT", self.num);
        self.bank0_size(buf.len() as u16);

        let mut bytes_sent = 0;
        while bytes_sent < buf.len() {
            self.desc.bank0.addr.write(|bank0| unsafe { bank0.addr().bits(buf.as_ptr() as u32 + bytes_sent as u32) });
            self.sync_tx(PToken::Out, None).await?;
            let sent = self.desc.bank0.pcksize.read().byte_count().bits() as usize;
            bytes_sent += sent;
        }

        trace!("USB Pipe[{}] Sent {} bytes", self.num, bytes_sent);
        Ok(bytes_sent)
    }

    async fn sync_tx(&mut self, token: PToken, until: Option<u64>) -> Result<(), PipeErr> {
        self.transfer_init(token);
        loop {
            // trace!("USB Pipe[{}] dispatching status", self.num);
            match self.transfer_status(token) {
                Ok(true) => {
                    return Ok(())
                }

                Err(err) => {
                    return Err(err)
                }

                Ok(false) =>
                    if let Some(timeout) = until {
                        if (self.millis)() > timeout {
                            return Err(PipeErr::SwTimeout);
                        }
                    }
            }
            // runtime::delay_cycles(50).await?;
        }
    }

    fn transfer_init(&mut self, token: PToken) {
        self.regs.cfg.modify(|_, w| unsafe { w.ptoken().bits(token as u8) });
        self.regs.intflag.modify(|_, w| w.trfail().set_bit());
        self.regs.intflag.modify(|_, w| w.perr().set_bit());

        match token {
            PToken::Setup => {
                self.regs.intflag.write(|w| w.txstp().set_bit());
                self.regs.statusset.write(|w| w.bk0rdy().set_bit());
            }
            PToken::In => {
                self.regs.statusclr.write(|w| w.bk0rdy().set_bit());
            }
            PToken::Out => {
                self.regs.intflag.write(|w| w.trcpt0().set_bit());
                self.regs.statusset.write(|w| w.bk0rdy().set_bit());
            }
            _ => {}
        }

        // self.trace_registers();
        self.regs.statusclr.write(|w| w.pfreeze().set_bit());
    }

    fn transfer_status(&mut self, token: PToken) -> Result<bool, PipeErr> {
        let intflag = self.regs.intflag.read();
        let status_pipe = self.desc.bank0.status_pipe.read();

        match token {
            PToken::Setup if intflag.txstp().bit_is_set() => {
                self.regs.intflag.write(|w| w.txstp().set_bit());
                return Ok(true);
            }
            PToken::In | PToken::Out if intflag.trcpt0().bit_is_set() => {
                self.regs.intflag.write(|w| w.trcpt0().set_bit());
                return Ok(true);
            }
            _ => {}
        };

        if status_pipe.ercnt().bits() > 0 {
            warn!("USB Pipe error bits {}", status_pipe.ercnt().bits())
        }

        if self.desc.bank0.status_bk.read().errorflow().bit_is_set() {
            // TODO nak is for bulk & interrupt, report overflow & underflow for isochronous ep
            return Err(PipeErr::Nak);
        }

        if intflag.trfail().bit_is_set() {
            self.regs.intflag.write(|w| w.trfail().set_bit());
            return Err(PipeErr::TransferFail);
        }
        if intflag.stall().bit_is_set() {
            self.regs.intflag.write(|w| w.stall().set_bit());
            return Err(PipeErr::Stall);
        }

        if status_pipe.dtgler().bit_is_set() {
            return Err(PipeErr::DataToggle);
        }
        if status_pipe.touter().bit_is_set() {
            return Err(PipeErr::HwTimeout);
        }

        Ok(false)
    }
}

// TODO: merge into SVD for pipe cfg register.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum PToken {
    Setup = 0x0,
    In = 0x1,
    Out = 0x2,
    // _Reserved = 0x3,
}

// TODO: merge into SVD for pipe cfg register.
#[allow(unused)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum PipeType {
    Disabled = 0x0,
    Control = 0x1,
    ISO = 0x2,
    Bulk = 0x3,
    Interrupt = 0x4,
    Extended = 0x5,
    // _Reserved0 = 0x06,
    // _Reserved1 = 0x07,
}

impl From<TransferType> for PipeType {
    fn from(v: TransferType) -> Self {
        match v {
            TransferType::Control => Self::Control,
            TransferType::Isochronous => Self::ISO,
            TransferType::Bulk => Self::Bulk,
            TransferType::Interrupt => Self::Interrupt,
        }
    }
}

// §32.8.7.1
pub(crate) struct PipeDesc {
    pub bank0: BankDesc,
    // can be used in ping-pong mode (SAMD USB dual buffering)
    pub bank1: BankDesc,
}

// 2 banks: 32 bytes per pipe.
impl PipeDesc {
    pub fn new() -> Self {
        Self {
            bank0: BankDesc::new(),
            bank1: BankDesc::new(),
        }
    }
}

#[repr(C)]
// 16 bytes per bank.
pub(crate) struct BankDesc {
    pub addr: Addr,
    pub pcksize: PckSize,
    pub extreg: ExtReg,
    pub status_bk: StatusBk,
    _reserved0: u8,
    pub ctrl_pipe: CtrlPipe,
    pub status_pipe: StatusPipe,
    _reserved1: u8,
}

impl BankDesc {
    fn new() -> Self {
        Self {
            addr: Addr::from(0),
            pcksize: PckSize::from(0),
            extreg: ExtReg::from(0),
            status_bk: StatusBk::from(0),
            _reserved0: 0,
            ctrl_pipe: CtrlPipe::from(0),
            status_pipe: StatusPipe::from(0),
            _reserved1: 0,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn bank_desc_sizes() {
        assert_eq!(core::mem::size_of::<Addr>(), 4, "Addr register size.");
        assert_eq!(core::mem::size_of::<PckSize>(), 4, "PckSize register size.");
        assert_eq!(core::mem::size_of::<ExtReg>(), 2, "ExtReg register size.");
        assert_eq!(
            core::mem::size_of::<StatusBk>(),
            1,
            "StatusBk register size."
        );
        assert_eq!(
            core::mem::size_of::<CtrlPipe>(),
            2,
            "CtrlPipe register size."
        );
        assert_eq!(
            core::mem::size_of::<StatusPipe>(),
            1,
            "StatusPipe register size."
        );

        // addr at 0x00 for 4
        // pcksize at 0x04 for 4
        // extreg at 0x08 for 2
        // status_bk at 0x0a for 2
        // ctrl_pipe at 0x0c for 2
        // status_pipe at 0x0e for 1
        assert_eq!(
            core::mem::size_of::<BankDesc>(),
            16,
            "Bank descriptor size."
        );
    }

    #[test]
    fn bank_desc_offsets() {
        let bd = BankDesc::new();
        let base = &bd as *const _ as usize;

        assert_offset("Addr", &bd.addr, base, 0x00);
        assert_offset("PckSize", &bd.pcksize, base, 0x04);
        assert_offset("ExtReg", &bd.extreg, base, 0x08);
        assert_offset("StatusBk", &bd.status_bk, base, 0x0a);
        assert_offset("CtrlPipe", &bd.ctrl_pipe, base, 0x0c);
        assert_offset("StatusPipe", &bd.status_pipe, base, 0x0e);
    }

    #[test]
    fn pipe_desc_size() {
        assert_eq!(core::mem::size_of::<PipeDesc>(), 32);
    }

    #[test]
    fn pipe_desc_offsets() {
        let pd = PipeDesc::new();
        let base = &pd as *const _ as usize;

        assert_offset("Bank0", &pd.bank0, base, 0x00);
        assert_offset("Bank1", &pd.bank1, base, 0x10);
    }

    fn assert_offset<T>(name: &str, field: &T, base: usize, offset: usize) {
        let ptr = field as *const _ as usize;
        assert_eq!(ptr - base, offset, "{} register offset.", name);
    }
}
