//! Depacking a crunched Commodore 64 program (SQ-1488).
//!
//! Some C64 releases distribute their one program crunched behind a `$0801`
//! BASIC loader stub: the stub's `SYS <addr>` jumps into the depacker, which
//! decompresses the real program over itself in memory and then jumps to it.
//! A disk holding one of these — the *Hulk* collection disks in
//! `stories/scott-dialects/c64/` are the specimens SQ-1488 exists for — has
//! no clear-text table for [`crate::d64`]'s directory walk to hand a Scott
//! Adams sniff, so the disk reads as empty until the program is depacked
//! first.
//!
//! This wraps `regenerator2000-core`'s 6502-emulation unpacker — see the
//! licence and provenance comment on the dependency in this crate's
//! `Cargo.toml` — which runs the depacker's own code against a simulated
//! 6502/C64 rather than recognising any particular cruncher by signature.

use regenerator2000_core::unpacker::{UnpackConfig, UnpackError, unpack};

/// The result of successfully depacking a crunched C64 program.
#[derive(Debug, Clone)]
pub struct DepackedProgram {
    /// The decompressed bytes, `data[0]` living at `start_addr`.
    pub data: Vec<u8>,
    /// The C64 memory address the decompressed data starts at.
    pub start_addr: u16,
    /// The entry point of the decompressed program (where execution would
    /// continue once the depacker hands off).
    pub entry_point: u16,
}

/// Why [`depack_c64_prg`] could not depack a program.
///
/// `regenerator2000-core` 0.9.20's own [`UnpackError`] does not name a
/// packer format it does not recognise — it only ever times out or reports
/// nothing moved — so there is no `UnsupportedFormat` case to carry here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepackError {
    /// The `PRG` carried no data past its load-address header — not a
    /// crunched program, or not a program at all.
    Empty,
    /// No BASIC `SYS` token was found in the loader stub, so this is not a
    /// `$0801` BASIC-stub program in the first place.
    NoEntryPoint,
    /// The emulator ran out of its instruction budget before the depacker's
    /// own code finished — almost always because `prg` was not actually a
    /// crunched program (nothing to decompress, so the CPU runs on into
    /// whatever bytes follow rather than exiting cleanly).
    TimedOut,
    /// The depacker ran and exited, but what came out was not a plausible
    /// decompressed program (no memory changed, or the reported entry point
    /// falls outside the range that changed).
    Failed,
}

/// Attempts to depack a Commodore 64 program via 6502 emulation.
///
/// `prg` is the file's raw bytes as a Commodore `PRG`: its first two bytes
/// are the little-endian load address, exactly as [`crate::d64::CbmFile`]
/// keeps them and as [`crate::medium::MountedDisk::contents`] hands them
/// back. On success, `DepackedProgram::data` is the decompressed memory
/// image, which a caller can hand to a byte-based format sniff exactly as it
/// would the original (still-packed) bytes.
pub fn depack_c64_prg(prg: &[u8]) -> Result<DepackedProgram, DepackError> {
    if prg.len() < 2 {
        return Err(DepackError::Empty);
    }
    let load_addr = u16::from_le_bytes([prg[0], prg[1]]);
    let raw = &prg[2..];
    if raw.is_empty() {
        return Err(DepackError::Empty);
    }
    let config = UnpackConfig::default();
    match unpack(raw, load_addr, &config, None) {
        Ok(result) => Ok(DepackedProgram {
            data: result.data,
            start_addr: result.start_addr,
            entry_point: result.entry_point,
        }),
        Err(UnpackError::EmptyData) => Err(DepackError::Empty),
        Err(UnpackError::NoEntryPoint) => Err(DepackError::NoEntryPoint),
        Err(UnpackError::Phase1Timeout | UnpackError::Phase2Timeout) => Err(DepackError::TimedOut),
        Err(UnpackError::NothingWritten | UnpackError::InvalidAddressRange { .. }) => {
            Err(DepackError::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_prg_is_rejected_without_reaching_the_emulator() {
        assert_eq!(depack_c64_prg(&[]).unwrap_err(), DepackError::Empty);
        assert_eq!(depack_c64_prg(&[0x01]).unwrap_err(), DepackError::Empty);
        assert_eq!(depack_c64_prg(&[0x01, 0x08]).unwrap_err(), DepackError::Empty);
    }

    /// A plain, uncrunched BASIC program — `10 PRINT"HI":GOTO10` — carries no
    /// `SYS` token at all, so this must be told apart from a crunched program
    /// rather than reported as one that merely failed to unpack.
    #[test]
    fn a_plain_basic_program_with_no_sys_token_reports_no_entry_point() {
        // `$0801`: link($080B), line 10, tokens `PRINT` `"HI"` `:` `GOTO` `10`, end-of-line, end-of-program.
        let prg: &[u8] = &[
            0x01, 0x08, // load address $0801
            0x0B, 0x08, // next line link
            0x0A, 0x00, // line number 10
            0x99, 0x22, b'H', b'I', 0x22, 0x3A, 0x89, 0x20, 0x31, 0x30, // PRINT"HI":GOTO 10
            0x00, // end of line
            0x00, 0x00, // end of program
        ];
        assert_eq!(depack_c64_prg(prg).unwrap_err(), DepackError::NoEntryPoint);
    }
}
