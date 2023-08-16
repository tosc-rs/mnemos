//! RISC-V trap/exception handling

// TODO(eliza): none of this is really D1-specific, and it should all work on
// any RISC-V target. If/when we add a generic `mnemos-riscv` crate, we may want
// to move this stuff there...

use core::fmt;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Trap {
    Interrupt(Interrupt),
    Exception(Exception),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidMcause {
    bits: usize,
    err: &'static str,
}

/// Pretty-prints a [`riscv_rt::TrapFrame`].
#[derive(Copy, Clone, Debug)]
pub struct PrettyTrapFrame<'a>(pub &'a riscv_rt::TrapFrame);

macro_rules! cause_enum {
    (
        $(#[$meta:meta])* $vis:vis enum $Enum:ident {
            $(
                $Variant:ident = $val:literal => $pretty:literal
            ),+
            $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(usize)]
        $vis enum $Enum {
            $(
                #[doc = $pretty]
                $Variant = $val,
            )+
        }

        impl fmt::Display for $Enum {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(
                        $Enum::$Variant => f.pad($pretty),
                    )+
                }
            }
        }

        impl fmt::UpperHex for $Enum {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::UpperHex::fmt(&(*self as usize), f)
            }
        }

        impl fmt::LowerHex for $Enum {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::LowerHex::fmt(&(*self as usize), f)
            }
        }

        impl TryFrom<usize> for $Enum {
            type Error = &'static str;

            fn try_from(value: usize) -> Result<Self, Self::Error> {
                match value {
                    $(
                        $val => Ok($Enum::$Variant),
                    )+
                    _ => Err(cause_enum!(@error: $Enum, $($val),+)),
                }
            }
        }
    };

    (@error: $Enum:ident, $firstval:literal, $($val:literal),*) => {
        concat!(
            "invalid value for ",
            stringify!($Enum),
            ", expected one of [",
            stringify!($firstval),
            $(
                ", ",
                stringify!($val),
            )+
            "]",
        )
    };
}

cause_enum! {
    /// RISC-V exception causes.
    ///
    /// If the interrupt bit (the highest bit) in the `mcause` register is 0, the
    /// rest of the `mcause` register is interpreted as an exception cause.
    ///
    /// See "3.1.20 Machine Cause Register (`mcause`)" in [_RISC-V Privelieged
    /// Architectures_, v1.10][manual].
    ///
    /// [manual]:
    ///     https://riscv.org/wp-content/uploads/2017/05/riscv-privileged-v1.10.pdf
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum Exception {
        InstAddrMisaligned = 0 => "Instruction address misaligned",
        InstAccessFault = 1 => "Instruction access fault",
        IllegalInst = 2 => "Illegal instruction",
        Breakpoint = 3 => "Breakpoint",
        LoadAddrMisaligned = 4 => "Load address misaligned",
        LoadAccessFault = 5 => "Load access fault",
        StoreAddrMisaligned = 6 => "Store/AMO address misaligned",
        StoreAccessFault = 7 => "Store/AMO access fault",
        UModeEnvCall = 8 => "Environment call from U-mode",
        SModeEnvCall = 9 => "Environment call from S-mode",
        MModeEnvCall = 11 => "Environment call from M-mode",
        InstPageFault = 12 => "Instruction page fault",
        LoadPageFault = 13 => "Load page fault",
        StorePageFault = 15 => "Store/AMO page fault",
    }
}

cause_enum! {
    /// RISC-V interrupt causes.
    ///
    /// If the interrupt bit (the highest bit) in the `mcause` register is 1, the
    /// rest of the `mcause` register is interpreted as an interrupt cause.
    ///
    /// See "3.1.20 Machine Cause Register (`mcause`)" in [_RISC-V Privelieged
    /// Architectures_, v1.10][manual].
    ///
    /// [manual]:
    ///     https://riscv.org/wp-content/uploads/2017/05/riscv-privileged-v1.10.pdf
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum Interrupt {
        UserSw = 0 => "User software interrupt",
        SupervisorSw = 1 => "Supervisor software interrupt",
        MachineSw = 3 => "Machine software interrupt",
        UserTimer = 4 => "User timer interrupt",
        SupervisorTimer = 5 => "Supervisor timer interrupt",
        MachineTimer = 7 => "Machine timer interrupt",
        UserExt = 8 => "User external interrupt",
        SupervisorExt = 9 => "Supervisor external interrupt",
        MachineExt = 11 => "Machine external interrupt",
    }
}

// === impl Trap ===

impl Trap {
    #[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
    pub fn from_mcause() -> Result<Self, InvalidMcause> {
        let mut bits: usize;
        unsafe {
            core::arch::asm!("csrrs {}, mcause, x0", out(reg) bits, options(nomem, nostack, preserves_flags));
        }
        Self::from_bits(bits)
    }

    #[cfg(not(any(target_arch = "riscv64", target_arch = "riscv32")))]
    pub fn from_mcause() -> Result<Self, InvalidMcause> {
        unimplemented!("cannot access mcause on a non-RISC-V platform!")
    }

    #[cfg_attr(
        not(any(target_arch = "riscv64", target_arch = "riscv32")),
        allow(dead_code)
    )]
    fn from_bits(bits: usize) -> Result<Self, InvalidMcause> {
        const INTERRUPT_BIT: usize = 1 << (usize::BITS - 1);

        let res = if bits & INTERRUPT_BIT == INTERRUPT_BIT {
            Interrupt::try_from(bits & !INTERRUPT_BIT).map(Self::Interrupt)
        } else {
            Exception::try_from(bits & !INTERRUPT_BIT).map(Self::Exception)
        };
        res.map_err(|err| InvalidMcause { err, bits })
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trap::Interrupt(t) => fmt::Display::fmt(t, f),
            Trap::Exception(t) => fmt::Display::fmt(t, f),
        }
    }
}

// === impl InvalidMcause ===

impl fmt::Display for InvalidMcause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { bits, err } = self;
        write!(f, "invalid mcause ({bits:#b}): {err}")
    }
}

const CHARS: usize = (usize::BITS / 4) as usize;

macro_rules! pretty_trap_frame {
    (fmt: $f:ident, frame: $frame:expr, $spec:literal => $($reg:ident),+ $(,)?) => {

        let nl = if $f.alternate() { "\n" } else { ", " };
        $(
            $f.pad(stringify!($reg))?;
            write!($f, concat!(": {:0width$", $spec,"}{}"), $frame.$reg, nl, width = CHARS)?;
        )+
    }
}

impl fmt::LowerHex for PrettyTrapFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        pretty_trap_frame!(fmt: f, frame: self.0, "x" => ra, t0, t1, t2, t3, t4, t5, t6, a0, a1, a2, a3, a4, a5, a6, a7);
        Ok(())
    }
}

impl fmt::UpperHex for PrettyTrapFrame<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        pretty_trap_frame!(fmt: f, frame: self.0, "X" => ra, t0, t1, t2, t3, t4, t5, t6, a0, a1, a2, a3, a4, a5, a6, a7);
        Ok(())
    }
}
