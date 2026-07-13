//! Small persistent state kept across `software_reset()`.
//!
//! These live in RTC-fast RAM with the `persistent` attribute: the runtime
//! zeroes them once on cold power-on, then preserves them across warm resets
//! (software reset, watchdog, etc.). We use them to recover from bad Wi-Fi
//! credentials without a human, and to carry a factory-reset request from the
//! button task to the next boot.

/// Consecutive failed STA connection attempts, counted across reboots.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut STA_FAIL_COUNT: u32 = 0;

/// Non-zero when the button task has requested a credential wipe on next boot.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut FACTORY_RESET: u32 = 0;

pub fn sta_fail_count() -> u32 {
    unsafe { (&raw const STA_FAIL_COUNT).read() }
}

pub fn set_sta_fail_count(v: u32) {
    unsafe { (&raw mut STA_FAIL_COUNT).write(v) }
}

/// Request a factory reset (credential wipe) on the next boot.
pub fn request_factory_reset() {
    unsafe { (&raw mut FACTORY_RESET).write(1) }
}

/// Returns whether a factory reset was requested, clearing the flag.
pub fn take_factory_reset() -> bool {
    unsafe {
        let requested = (&raw const FACTORY_RESET).read() != 0;
        (&raw mut FACTORY_RESET).write(0);
        requested
    }
}
