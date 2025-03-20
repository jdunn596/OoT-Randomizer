#![no_std]
#![no_builtins]

use core::panic::PanicInfo;

#[cfg_attr(target_vendor = "ultra64", panic_handler)]
#[unsafe(no_mangle)] fn panic(_: &PanicInfo<'_>) -> ! {
    loop {}
}

#[unsafe(no_mangle)] pub extern "C" fn rust_add(left: u32, right: u32) -> u32 {
    left.wrapping_add(right)
}
