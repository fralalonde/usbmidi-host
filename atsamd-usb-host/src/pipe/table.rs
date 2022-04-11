use crate::pipe::{Pipe, PipeDesc, PipeType};
use crate::pipe::regs::PipeRegs;
use usb_host::Endpoint;
use atsamd_hal::target_device::usb;

// SAMD21 only supports 8 pipes
pub(crate) const MAX_PIPES: usize = 8;

pub struct PipeTable {
    tbl: [PipeDesc; MAX_PIPES],
}

impl PipeTable {
    pub(crate) fn new() -> Self {
        let tbl = {
            let mut tbl: [core::mem::MaybeUninit<PipeDesc>; MAX_PIPES] =
                unsafe { core::mem::MaybeUninit::uninit().assume_init() };

            for e in &mut tbl[..] {
                unsafe { core::ptr::write(e.as_mut_ptr(), PipeDesc::new()) }
            }

            unsafe { core::mem::transmute(tbl) }
        };
        Self { tbl }
    }

    pub(crate) fn pipe_for<'a, 'b>(&'a mut self, host: &'b mut usb::HOST, endpoint: &dyn Endpoint, millis: fn() -> u64) -> Pipe<'a, 'b> {
        // Pipe 0 is always for control endpoints, 1 for everything else (for now)
        // TODO: cache in-use pipes and return them without init?
        let pnum = if endpoint.endpoint_address() == 0 { 0 } else { 1 };

        let pipe_regs = PipeRegs::from(host, pnum);
        pipe_regs.cfg.write(|w| {
            let ptype = PipeType::from(endpoint.transfer_type());
            unsafe { w.ptype().bits(ptype as u8) }
        });

        let pdesc = &mut self.tbl[pnum];
        pdesc.bank0.ctrl_pipe.write(|w| {
            w.pdaddr().set_addr(endpoint.device_address().into());
            w.pepnum().set_epnum(endpoint.endpoint_address())
        });
        pdesc.bank0.pcksize.write(|w| {
            let mps = endpoint.max_packet_size();
            if mps >= 1023 {
                w.size().bytes1024()
            } else if mps >= 512 {
                w.size().bytes512()
            } else if mps >= 256 {
                w.size().bytes256()
            } else if mps >= 128 {
                w.size().bytes128()
            } else if mps >= 64 {
                w.size().bytes64()
            } else if mps >= 32 {
                w.size().bytes32()
            } else if mps >= 16 {
                w.size().bytes16()
            } else {
                w.size().bytes8()
            }
        });
        Pipe {
            num: pnum,
            regs: pipe_regs,
            desc: pdesc,
            millis,
        }
    }
}
