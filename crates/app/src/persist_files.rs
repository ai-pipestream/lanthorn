use std::path::Path;
use mapper::mapper::Mapper;
use mapper::persist::{to_json, from_json};

pub fn save_map(path: &Path, mapper: &Mapper) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, to_json(mapper))
}

pub fn load_map(path: &Path) -> Option<Mapper> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return None,
    };
    match from_json(&contents) {
        Ok(mapper) => Some(mapper),
        Err(e) => {
            eprintln!("babelmap: failed to parse map file {}: {}", path.display(), e);
            None
        }
    }
}

pub fn save_game(path: &Path, machine: &zvm::cpu::exec::Machine) -> std::io::Result<()> {
    std::fs::write(path, machine.save_quetzal())
}

pub fn restore_game(path: &Path, machine: &mut zvm::cpu::exec::Machine) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    machine.restore_quetzal(&bytes).map_err(|e| match e {
        zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
        other => format!("restore failed: {:?}", other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::mapper::Mapper;
    use mapper::direction::Direction;

    #[test]
    fn save_then_load_round_trips() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("babelmap-test-{}", std::process::id()));
        let path = dir.join("ZCODE-1-x-0.map.json");
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        save_map(&path, &m).unwrap();
        let loaded = load_map(&path).expect("loads");
        assert_eq!(loaded.graph.connections(), m.graph.connections());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_is_none() {
        assert!(load_map(Path::new("/no/such/babelmap.map.json")).is_none());
    }

    #[test]
    fn load_corrupt_is_none() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("babelmap-test-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.map.json");
        std::fs::write(&path, b"this is not valid json {{{").unwrap();
        assert!(load_map(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_save_restore_round_trips_with_czech() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(story) = std::fs::read(&fixture) else { return /* skip */ };
        let mem = zvm::memory::Memory::new(story).unwrap();
        let mut machine = zvm::cpu::exec::Machine::new(mem);
        machine.init_caps();
        // step a few instructions so dynamic memory differs from the pristine image
        for _ in 0..50 { let _ = machine.step(); }
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("babelmap-save-{}.qzl", std::process::id()));
        save_game(&tmp, &machine).unwrap();
        let mut m2 = zvm::cpu::exec::Machine::new(
            zvm::memory::Memory::new(std::fs::read(&fixture).unwrap()).unwrap()
        );
        m2.init_caps();
        restore_game(&tmp, &mut m2).expect("restore ok");
        let _ = std::fs::remove_file(&tmp);
    }
}
