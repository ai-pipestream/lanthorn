// gvm-cli — a headless Glulx runner.
//
// Usage: gvm-cli <story.ulx | story.gblorb>
//
// Loads a raw `.ulx` Glulx image or extracts the `GLUL` executable from a Blorb,
// runs it to completion (Phase 2a is non-interactive), streams output to stdout,
// and prints any diagnostics to stderr.

use std::any::Any;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

use gvm::{Machine, Memory, Output};

/// Output sink that writes directly to stdout.
struct StdoutOutput;

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) {
        print!("{s}");
        let _ = io::stdout().flush();
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Get the runnable Glulx image: the `GLUL` executable inside a Blorb, or the
/// raw bytes when they aren't a Blorb (a plain `.ulx`).
fn extract_executable(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if !blorb::Blorb::is_blorb(&bytes) {
        return Ok(bytes);
    }
    let b = blorb::Blorb::parse(bytes).map_err(|e| format!("Error: invalid Blorb: {e:?}"))?;
    match b.executable() {
        Ok((blorb::ExecKind::Glulx, data)) => Ok(data.to_vec()),
        Ok((blorb::ExecKind::ZCode, _)) => {
            Err("Error: this is a Z-code Blorb; run it with zvm-cli.".to_string())
        }
        Err(e) => Err(format!("Error: Blorb has no executable: {e:?}")),
    }
}

/// Build a machine from story bytes, sending output to `out`.
fn build_machine(bytes: Vec<u8>, out: Box<dyn Output>) -> Result<Machine, String> {
    let image = extract_executable(bytes)?;
    let mem = Memory::new(image).map_err(|e| format!("Error loading Glulx image: {e:?}"))?;
    Ok(Machine::with_output(mem, out))
}

fn main() {
    let argv: Vec<String> = env::args().collect();
    let Some(path) = argv.get(1) else {
        eprintln!("Usage: {} <story.ulx | story.gblorb>", argv[0]);
        process::exit(1);
    };

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: cannot read '{path}': {e}");
            process::exit(1);
        }
    };

    let mut machine = match build_machine(bytes, Box::new(StdoutOutput)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    machine.run();

    for d in &machine.diagnostics {
        eprintln!("gvm: {d}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gvm::BufferOutput;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A hand-assembled Glulx image: print "Hi" then quit.
    fn hi_image() -> Vec<u8> {
        // Start function (at 0x24): setiosys(2,0); streamchar 'H'; streamchar 'i'; quit.
        let func: Vec<u8> = vec![
            0xC1, 0x00, 0x00, // type C1, empty locals format
            0x81, 0x49, 0x11, 0x02, 0x00, // setiosys glk, rock 0
            0x70, 0x01, b'H', // streamchar 'H'
            0x70, 0x01, b'i', // streamchar 'i'
            0x81, 0x20, // quit
        ];
        let (ramstart, endmem) = (0x100u32, 0x200u32);
        let mut img = vec![0u8; ramstart as usize];
        img[0..4].copy_from_slice(b"Glul");
        img[0x04..0x08].copy_from_slice(&0x0003_0102u32.to_be_bytes());
        img[0x08..0x0C].copy_from_slice(&ramstart.to_be_bytes());
        img[0x0C..0x10].copy_from_slice(&ramstart.to_be_bytes()); // EXTSTART == RAMSTART
        img[0x10..0x14].copy_from_slice(&endmem.to_be_bytes());
        img[0x14..0x18].copy_from_slice(&0x1000u32.to_be_bytes());
        img[0x18..0x1C].copy_from_slice(&0x24u32.to_be_bytes());
        img[0x24..0x24 + func.len()].copy_from_slice(&func);
        img
    }

    // ── minimal Blorb builder (mirrors the blorb crate's test helper) ─────────
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    /// A Blorb wrapping a single Exec resource of the given chunk type + data.
    fn build_blorb(exec_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let ridx_data_len = 4 + 12; // count + one 12-byte entry
        let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
        let exec_chunk = chunk(exec_type, data);

        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // count
        ridx.extend_from_slice(b"Exec");
        ridx.extend_from_slice(&0u32.to_be_bytes()); // number
        ridx.extend_from_slice(&(first_res_off as u32).to_be_bytes());
        let ridx_chunk = chunk(b"RIdx", &ridx);

        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&ridx_chunk);
        inner.extend_from_slice(&exec_chunk);
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    /// Sink that captures output into a shared buffer (the machine's own sink is
    /// not reachable from this crate).
    struct CaptureSink(Rc<RefCell<String>>);
    impl Output for CaptureSink {
        fn print(&mut self, s: &str) {
            self.0.borrow_mut().push_str(s);
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn run_capturing(bytes: Vec<u8>) -> String {
        let buf = Rc::new(RefCell::new(String::new()));
        let mut m = build_machine(bytes, Box::new(CaptureSink(buf.clone()))).unwrap();
        m.run();
        let out = buf.borrow().clone();
        out
    }

    #[test]
    fn runs_raw_ulx_to_completion() {
        assert_eq!(run_capturing(hi_image()), "Hi");
    }

    #[test]
    fn extracts_and_runs_glulx_blorb() {
        let blorb = build_blorb(b"GLUL", &hi_image());
        assert!(blorb::Blorb::is_blorb(&blorb));
        assert_eq!(run_capturing(blorb), "Hi");
    }

    #[test]
    fn rejects_zcode_blorb() {
        let zblorb = build_blorb(b"ZCOD", b"fake z-code");
        let err = match build_machine(zblorb, Box::new(BufferOutput::new())) {
            Err(e) => e,
            Ok(_) => panic!("expected the Z-code Blorb to be rejected"),
        };
        assert!(err.contains("Z-code"), "got: {err}");
    }

    #[test]
    fn passes_through_non_blorb_bytes() {
        let img = hi_image();
        assert_eq!(extract_executable(img.clone()).unwrap(), img);
    }
}
