pub use zvm::ifid::compute_ifid;

pub fn map_path(base_dir: &std::path::Path, ifid: &str) -> std::path::PathBuf {
    base_dir.join(format!("{ifid}.map.json"))
}

pub fn archive_path(base_dir: &std::path::Path, ifid: &str) -> std::path::PathBuf {
    base_dir.join(format!("{ifid}.babelmap"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn map_path_uses_ifid() {
        let p = map_path(Path::new("/tmp/maps"), "ZCODE-42-871124-ABCD");
        assert_eq!(p, Path::new("/tmp/maps/ZCODE-42-871124-ABCD.map.json"));
    }

    #[test]
    fn archive_path_uses_ifid() {
        let p = archive_path(Path::new("/tmp/maps"), "ZCODE-42-871124-ABCD");
        assert_eq!(p, Path::new("/tmp/maps/ZCODE-42-871124-ABCD.babelmap"));
    }
}
