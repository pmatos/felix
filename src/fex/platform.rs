// SPDX-License-Identifier: MIT

/// Returns the frequency of the hardware cycle counter.
///
/// On aarch64, reads `CNTFRQ_EL0`. On `x86_64`, returns 1 (cycle counter
/// frequency is not directly readable).
#[must_use]
pub fn cycle_counter_frequency() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        let freq: u64;
        // SAFETY: CNTFRQ_EL0 is always readable from userspace on aarch64.
        unsafe {
            std::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack));
        }
        freq
    }
    #[cfg(target_arch = "x86_64")]
    {
        1
    }
}

/// Issues a store memory barrier visible to the inner-shareable domain.
///
/// On aarch64, executes `dmb ishst`. On `x86_64`, this is a no-op because
/// x86 has strong memory ordering for stores.
pub fn store_memory_barrier() {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: dmb ishst is a barrier instruction with no side effects
        // beyond ordering memory operations.
        unsafe {
            std::arch::asm!("dmb ishst", options(nostack));
        }
    }
    #[cfg(target_arch = "x86_64")]
    {}
}
