//! Local, CPU-only sentence embeddings. Only pinned model files are fetched;
//! the query and transcript never leave this process.
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokenizers::{Encoding, Tokenizer, TruncationParams};

const DIM: usize = 384;

#[derive(serde::Deserialize)]
struct ModelManifest {
    model: String,
    revision: String,
    files: Vec<ModelArtifact>,
}

#[derive(serde::Deserialize)]
struct ModelArtifact {
    name: String,
    sha256: String,
    size: u64,
}

fn manifest() -> &'static ModelManifest {
    static MANIFEST: std::sync::OnceLock<ModelManifest> = std::sync::OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("model-manifest.json"))
            .expect("bundled model manifest must be valid")
    })
}

pub(super) struct SentenceEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    pool: rayon::ThreadPool,
}

impl SentenceEmbedder {
    pub(super) fn load() -> Result<Self, String> {
        let override_dir = std::env::var_os("LANTHORN_RECALL_MODEL_DIR").map(PathBuf::from);
        let directory = match &override_dir {
            Some(path) => path.clone(),
            None => cache_dir()?
                .join("lanthorn")
                .join("recall")
                .join(&manifest().revision),
        };
        Self::load_from(&directory, override_dir.is_none())
    }

    fn load_from(directory: &Path, download: bool) -> Result<Self, String> {
        if download {
            std::fs::create_dir_all(directory).map_err(|e| format!("model cache: {e}"))?;
        }
        let mut contents = Vec::new();
        for file in &manifest().files {
            contents.push(model_file(
                directory,
                &file.name,
                &file.sha256,
                file.size,
                download,
            )?);
        }
        let config: Config = serde_json::from_slice(&contents[0]).map_err(|e| e.to_string())?;
        let mut tokenizer = Tokenizer::from_bytes(&contents[1]).map_err(|e| e.to_string())?;
        tokenizer.with_padding(None);
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 256,
                stride: 32,
                ..Default::default()
            }))
            .map_err(|e| e.to_string())?;
        let weights = contents.pop().expect("three model artifacts");
        // Buffered safetensors avoids an unsafe memory map of a mutable cache file.
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu)
            .map_err(|e| e.to_string())?;
        let model = BertModel::load(vb, &config).map_err(|e| e.to_string())?;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .thread_name(|i| format!("recall-embedding-{i}"))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            model,
            tokenizer,
            pool,
        })
    }

    pub(super) fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.pool
            .install(|| texts.iter().map(|text| self.embed_text(text)).collect())
    }

    fn embed_text(&self, text: &str) -> Result<Vec<f32>, String> {
        if text.trim().is_empty() {
            return Err("cannot embed empty text".into());
        }
        let encoded = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| e.to_string())?;
        let mut combined = vec![0.0; DIM];
        // Tokenizer overflow includes the entire tail. A long token-heavy passage
        // must not silently lose evidence after the model's 256-token window.
        let windows = std::iter::once(&encoded).chain(encoded.get_overflowing().iter());
        for window in windows {
            let vector = self.embed_window(window).map_err(|e| e.to_string())?;
            for (sum, value) in combined.iter_mut().zip(vector) {
                *sum += value;
            }
        }
        normalize(&mut combined)?;
        Ok(combined)
    }

    fn embed_window(&self, encoded: &Encoding) -> candle_core::Result<Vec<f32>> {
        let ids = Tensor::new(encoded.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
        let mask = Tensor::new(encoded.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
        let output = self.model.forward(&ids, &ids.zeros_like()?, Some(&mask))?;
        // Sentence Transformers' masked mean pooling, then L2 normalization.
        let mask = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let pooled = output
            .broadcast_mul(&mask)?
            .sum(1)?
            .broadcast_div(&mask.sum(1)?)?;
        let normalized = pooled.broadcast_div(&pooled.sqr()?.sum_keepdim(1)?.sqrt()?)?;
        normalized.squeeze(0)?.to_vec1()
    }
}

fn normalize(values: &mut [f32]) -> Result<(), String> {
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err("embedding has no finite direction".into());
    }
    for v in values {
        *v /= norm;
    }
    Ok(())
}

fn cache_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|s| !s.is_empty()) {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    if cfg!(windows) {
        if let Some(dir) = std::env::var_os("LOCALAPPDATA").filter(|s| !s.is_empty()) {
            return Ok(PathBuf::from(dir));
        }
    }
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(|p| PathBuf::from(p).join(".cache"))
        .ok_or_else(|| "no model cache directory; set LANTHORN_RECALL_MODEL_DIR".into())
}

fn verified(bytes: &[u8], hash: &str, size: u64) -> bool {
    bytes.len() as u64 == size && format!("{:x}", Sha256::digest(bytes)) == hash
}

fn read_bounded(path: &Path, size: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(size + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn model_file(
    dir: &Path,
    name: &str,
    hash: &str,
    size: u64,
    download: bool,
) -> Result<Vec<u8>, String> {
    let path = dir.join(name);
    if let Ok(bytes) = read_bounded(&path, size) {
        if verified(&bytes, hash, size) {
            return Ok(bytes);
        }
    }
    if !download {
        return Err(format!(
            "{name} missing or invalid in {}; expected pinned MiniLM model",
            dir.display()
        ));
    }
    let model = manifest();
    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{name}",
        model.model, model.revision
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(300)))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("download {name}: {e}"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("download {name}: {e}"))?;
    if !verified(&bytes, hash, size) {
        return Err(format!("{name} failed model integrity check"));
    }
    // Concurrent sessions can safely install the same verified artifact. Never
    // leave a partially written final filename after interruption.
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    tmp.write_all(&bytes).map_err(|e| e.to_string())?;
    tmp.persist(&path).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(all(test, feature = "t-guidance"))]
mod tests {
    use super::*;

    #[test]
    fn rejects_nonfinite_and_empty_vectors() {
        for mut values in [vec![], vec![0.0, 0.0], vec![f32::NAN], vec![f32::INFINITY]] {
            assert!(normalize(&mut values).is_err());
        }
        let mut vector = [3.0, 4.0];
        normalize(&mut vector).unwrap();
        assert!((vector[0] - 0.6).abs() < 1e-6);
        assert!((vector[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn artifact_validation_requires_size_and_digest() {
        let digest = format!("{:x}", Sha256::digest(b"model"));
        assert!(verified(b"model", &digest, 5));
        assert!(!verified(b"model", &digest, 4));
        assert!(!verified(b"other", &digest, 5));
    }

    #[test]
    #[ignore = "downloads pinned 91 MB model; explicit real-model acceptance check"]
    fn real_model_semantics_and_overflow() {
        let mut embedder = SentenceEmbedder::load().unwrap();
        let texts = [
            "What can I use to illuminate a dark room?",
            "The brass lantern casts a bright light when you turn it on.",
            "The locked chest contains three gold coins.",
            "A sailor offers you passage across the ocean.",
        ]
        .map(String::from);
        let start = std::time::Instant::now();
        let vectors = embedder.embed(&texts).unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        for vector in &vectors {
            assert_eq!(vector.len(), DIM);
            assert!((dot(vector, vector) - 1.0).abs() < 1e-5);
        }
        assert!(dot(&vectors[0], &vectors[1]) > dot(&vectors[0], &vectors[2]));
        assert!(dot(&vectors[0], &vectors[1]) > dot(&vectors[0], &vectors[3]));
        let long = format!("{} {}", "The courtyard is empty. ".repeat(100), texts[1]);
        let encoded = embedder.tokenizer.encode(long.as_str(), true).unwrap();
        assert!(!encoded.get_overflowing().is_empty());
        let tail = encoded.get_overflowing().last().unwrap();
        let decoded = embedder.tokenizer.decode(tail.get_ids(), true).unwrap();
        assert!(decoded.contains("lantern"), "tail was lost: {decoded}");
        assert_eq!(embedder.embed(&[long]).unwrap()[0].len(), DIM);
        eprintln!(
            "real MiniLM: 4 passages plus overflow in {:?}; lantern similarity {}",
            start.elapsed(),
            dot(&vectors[0], &vectors[1])
        );
    }
}
