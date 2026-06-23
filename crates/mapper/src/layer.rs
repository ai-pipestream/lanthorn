//! Map layers ("segments"): a manual organizing tool. Every room belongs to exactly
//! one layer (default `MAIN_LAYER`). Layers are created/destroyed only by explicit
//! peel/merge — never derived. See docs/superpowers/specs/2026-06-23-manual-map-layers-design.md.

/// Stable layer identifier. Layer `0` (`MAIN_LAYER`) always exists.
pub type LayerId = u16;

/// The permanent base layer every room starts in.
pub const MAIN_LAYER: LayerId = 0;

/// Per-layer metadata: a display name and the layer it was peeled from (for merge default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayerMeta {
    pub name: String,
    pub parent: Option<LayerId>,
}

impl LayerMeta {
    /// Metadata for the base "Main" layer.
    pub fn main() -> Self {
        LayerMeta { name: "Main".to_string(), parent: None }
    }
}
