

use defmt_rtt as _;

extern crate panic_probe as _;

defmt::timestamp!("{=u64}", {
    crate::time::now_millis()
});

#[cfg(all(target_arch = "arm", target_os = "none"))]
#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}
