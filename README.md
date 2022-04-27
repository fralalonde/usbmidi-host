# USB MIDI Host adapter

Acts as host for a single USB MIDI device, converting signals to high speed serial MIDI.

_Under development._

## Notes

Intended to be used as companion board for https://github.com/fralalonde/usbmidi-route

## Platform:

- Adafruit Trinket M0 (Atmel SAM D21)
- Rust

Uses async-modified versions of the `usb-host` and `atsamd-usb-host` crates.

The `runtime` lib provides a custom Rust async implementation compatible with RTIC, based on the `woke` crate.

The `sync-thumbv6m` lib provides original and mutated data structures adapted to the Cortex-M0 lack of atomic operations.

## Hardware setup

Connect to Trinket M0 board using
underside [SWD pads](https://learn.adafruit.com/adafruit-trinket-m0-circuitpython-arduino/pinouts#secret-swd-pads-2910533-6)

For the probe I use a [bluepill](https://stm32-base.org/boards/STM32F103C8T6-Blue-Pill.html)

Then I flash it with [DAP42 firmware](https://github.com/devanlai/dap42)

Very simple wiring turns it into
an [SWD probe](https://microcontrollerelectronics.com/turn-an-stm32f103c8t6-blueplll-into-an-stlink-programmer/)

STLinkV2-compatible probes also work... until they don't (why?)

## Software setup

If not already installed, run

    cargo install probe-run

And then

    cargo build
    probe-run --chip ATSAMD21E17A target/thumbv6m-none-eabi/debug/usb-serial

If it doesn't work the first time... retry more.

Note: `chip` type for SAM D21 on Trinket M0 _should_ be **ATSAMD21E18A** but actually needs to be **ATSAMD21E17A**.
See https://github.com/probe-rs/probe-rs/issues/507

### Alternate setup for combined IDE debugging + logging

Instead of probe-rs, Use OpenOCD to flash the app and run as GDB server _and_ RTT/defmt server

See the `watch_rtt.sh` script or just use CLion run configs from `.run` dir

(Derived from https://ferrous-systems.com/blog/gdb-and-defmt/)

## License

Licensed under the terms of the Apache 2.0 and MIT license.