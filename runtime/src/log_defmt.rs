

use defmt_rtt as _;

extern crate panic_probe as _;

defmt::timestamp!("{=u64}", {
    crate::time::now_millis()
});

#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
