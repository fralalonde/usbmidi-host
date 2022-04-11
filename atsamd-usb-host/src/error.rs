use runtime::RuntimeError;
use usb_host::TransferError;

impl From<PipeErr> for TransferError {
    fn from(v: PipeErr) -> Self {
        match v {
            PipeErr::TransferFail => Self::Permanent("Transfer failed"),
            PipeErr::Nak => Self::Retry("NAK"),
            PipeErr::Underflow => Self::Retry("Underflow"),
            PipeErr::Overflow => Self::Retry("Overflow"),
            PipeErr::DataToggle => Self::Retry("Toggle sequence"),

            PipeErr::Stall => Self::Permanent("Pipe: Stall"),
            // PipeErr::PipeErr => Self::Permanent("Pipe error"),
            PipeErr::HwTimeout => Self::Permanent("Pipe: Hardware timeout"),
            PipeErr::SwTimeout => Self::Permanent("Pipe: Software timeout"),
            // PipeErr::Other(s) => Self::Permanent(s),
            PipeErr::Runtime(err) => Self::Permanent("Pipe: Interrupted"),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, defmt::Format)]
#[allow(unused)]
pub(crate) enum PipeErr {
    Runtime(RuntimeError),
    Stall,
    TransferFail,
    // PipeErr,
    Nak,
    Overflow,
    Underflow,
    HwTimeout,
    DataToggle,
    SwTimeout,
    // Other(&'static str),
}

impl From<runtime::RuntimeError> for PipeErr {
    fn from(err: RuntimeError) -> Self {
        PipeErr::Runtime(err)
    }
}

// impl From<&'static str> for PipeErr {
//     fn from(v: &'static str) -> Self {
//         Self::Other(v)
//     }
// }
