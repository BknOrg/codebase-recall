//! Pinned, verified local models. Only `setup` performs network I/O.
use super::{Embedder, REVISION};
use crate::config::{EmbeddingsConfig, ProjectConfig};
use anyhow::{Context, Result, ensure};
use fastembed::{
    InitOptionsUserDefined, OutputKey, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
const REPO: &str = "onnx-community/embeddinggemma-300m-ONNX";
#[derive(serde::Deserialize)]
struct Artifact {
    file: String,
    sha256: String,
    bytes: u64,
}
fn manifest() -> Result<Vec<Artifact>> {
    Ok(serde_json::from_str(include_str!("manifest.json"))?)
}
fn directory(project: &Path, cfg: &EmbeddingsConfig) -> Result<PathBuf> {
    if let Some(p) = &cfg.model_dir {
        return Ok(if p.is_absolute() {
            p.clone()
        } else {
            project.join(p)
        });
    }
    Ok(dirs::cache_dir()
        .context("cannot determine model cache; configure [embeddings] model_dir")?
        .join("code-rcl/models")
        .join(format!("embeddinggemma-300m-q4-{REVISION}")))
}
fn checksum(path: &Path, artifact: &Artifact) -> Result<()> {
    let mut file = fs::File::open(path).with_context(|| {
        format!(
            "missing model file {}; run embeddings setup or configure model_dir",
            path.display()
        )
    })?;
    ensure!(
        file.metadata()?.len() == artifact.bytes,
        "model size mismatch: {}; run embeddings setup",
        path.display()
    );
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        crate::operation::check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    ensure!(
        format!("{:x}", hasher.finalize()) == artifact.sha256,
        "model checksum mismatch: {}; run embeddings setup",
        path.display()
    );
    Ok(())
}
fn verify(dir: &Path) -> Result<()> {
    let _timer = crate::operation::time("model_verification");
    for a in manifest()? {
        checksum(&dir.join(&a.file), &a)?;
    }
    Ok(())
}
pub fn verify_configured(project: &Path) -> Result<()> {
    crate::progress::phase("verifying_model");
    let cfg = ProjectConfig::load(project)?;
    let dir = directory(project, &cfg.embeddings)?
        .canonicalize()
        .context("model directory missing; run embeddings setup")?;
    verify(&dir)
}
pub fn setup(project: &Path, local: Option<&Path>) -> Result<serde_json::Value> {
    let cfg = ProjectConfig::load(project)?;
    let dir = if let Some(local) = local {
        local
            .canonicalize()
            .context("model directory does not exist")?
    } else {
        directory(project, &cfg.embeddings)?
    };
    if local.is_some() {
        verify(&dir)?;
    } else {
        for a in manifest()? {
            crate::operation::check()?;
            let path = dir.join(&a.file);
            if path.exists() && checksum(&path, &a).is_ok() {
                continue;
            }
            fs::create_dir_all(path.parent().unwrap())?;
            let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            let url = format!(
                "https://huggingface.co/{REPO}/resolve/{REVISION}/{}",
                a.file
            );
            eprintln!("downloading model file {}", a.file);
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .tls_config(
                    ureq::tls::TlsConfig::builder()
                        .provider(ureq::tls::TlsProvider::NativeTls)
                        .build(),
                )
                .timeout_global(Some(std::time::Duration::from_secs(1800)))
                .timeout_connect(Some(std::time::Duration::from_secs(15)))
                .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
                .timeout_recv_body(Some(std::time::Duration::from_secs(1800)))
                .build()
                .into();
            let mut response = agent
                .get(&url)
                .call()
                .with_context(|| format!("downloading {url}"))?;
            let mut reader = response.body_mut().as_reader();
            let mut buffer = [0u8; 65536];
            let mut size = 0;
            loop {
                crate::operation::check()?;
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                ensure!(size <= a.bytes, "model download exceeds expected size");
                temp.write_all(&buffer[..n])?;
            }
            temp.flush()?;
            checksum(temp.path(), &a)?;
            temp.persist(&path).map_err(|e| e.error)?;
        }
    }
    // Preserve all unrelated project configuration, including unknown future sections.
    fs::create_dir_all(project.join(".code-rcl"))?;
    let path = project.join(".code-rcl/config.toml");
    let mut doc: toml::Value = if path.exists() {
        toml::from_str(&fs::read_to_string(&path)?)?
    } else {
        toml::Value::Table(Default::default())
    };
    set_model_dir(&mut doc, &dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    tmp.write_all(toml::to_string_pretty(&doc)?.as_bytes())?;
    tmp.flush()?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(
        serde_json::json!({"model":super::MODEL,"revision":REVISION,"model_dir":dir,"verified":true,"next":"code-rcl embeddings sync"}),
    )
}
fn set_model_dir(doc: &mut toml::Value, dir: &Path) -> Result<()> {
    let root = doc
        .as_table_mut()
        .context("project configuration must be a table")?;
    let embeddings = root
        .entry("embeddings")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .context("embeddings configuration must be a table")?;
    embeddings.insert(
        "model_dir".into(),
        toml::Value::String(dir.to_string_lossy().into_owned()),
    );
    Ok(())
}
struct Runtime {
    model: TextEmbedding,
    tokenizer: tokenizers::Tokenizer,
}
impl Embedder for Runtime {
    fn input_tokens(&self, text: &str) -> Result<usize> {
        let _timer = crate::operation::time("input_tokenization");
        Ok(self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .len())
    }
    fn offsets(&self, text: &str) -> Result<Vec<(usize, usize)>> {
        let _timer = crate::operation::time("chunk_tokenization");
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(encoding
            .get_offsets()
            .iter()
            .copied()
            .filter(|(a, b)| b > a)
            .collect())
    }
    fn embed(&mut self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let _timer = crate::operation::time("inference");
        let batch_size = texts.len().max(1);
        Ok(self.model.embed(texts, Some(batch_size))?)
    }
}
static RUNTIMES: OnceLock<Mutex<HashMap<(PathBuf, usize), Runtime>>> = OnceLock::new();
pub fn with_model<T>(project: &Path, f: impl FnOnce(&mut dyn Embedder) -> Result<T>) -> Result<T> {
    let cfg = ProjectConfig::load(project)?;
    let dir = directory(project, &cfg.embeddings)?
        .canonicalize()
        .context("model directory missing; run embeddings setup")?;
    let mut cache = RUNTIMES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow::anyhow!("embedding runtime lock poisoned"))?;
    let threads = cfg.embeddings.effective_threads();
    crate::progress::update(|s| {
        s["threads_configured"] = serde_json::json!(cfg.embeddings.threads);
        s["threads_effective"] = serde_json::json!(threads);
    });
    let key = (dir.clone(), threads);
    if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(key.clone()) {
        crate::progress::phase("verifying_model");
        verify(&dir)?;
        crate::operation::check()?;
        crate::progress::phase("loading_model");
        let load_timer = crate::operation::time("model_load");
        let files = TokenizerFiles {
            tokenizer_file: fs::read(dir.join("tokenizer.json"))?,
            config_file: fs::read(dir.join("config.json"))?,
            special_tokens_map_file: fs::read(dir.join("special_tokens_map.json"))?,
            tokenizer_config_file: fs::read(dir.join("tokenizer_config.json"))?,
        };
        let mut def =
            UserDefinedEmbeddingModel::new(fs::read(dir.join("onnx/model_q4.onnx"))?, files)
                .with_external_initializer(
                    "model_q4.onnx_data".into(),
                    fs::read(dir.join("onnx/model_q4.onnx_data"))?,
                );
        def.output_key = Some(OutputKey::ByName("sentence_embedding"));
        let model = TextEmbedding::try_new_from_user_defined(
            def,
            InitOptionsUserDefined::new()
                .with_max_length(512)
                .with_intra_threads(threads),
        )
        .context("loading EmbeddingGemma 1 Q4 ONNX model")?;
        let mut tokenizer = model.tokenizer.clone();
        tokenizer
            .with_truncation(None)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        tokenizer.with_padding(None);
        entry.insert(Runtime { model, tokenizer });
        drop(load_timer);
    }
    crate::operation::check()?;
    f(cache.get_mut(&key).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires CODE_RCL_EMBEDDING_MODEL_DIR; loads the local model"]
    fn real_model_auto_and_explicit_effective_threads_share_runtime() {
        let model = std::env::var("CODE_RCL_EMBEDDING_MODEL_DIR").unwrap();
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".code-rcl")).unwrap();
        let path = d.path().join(".code-rcl/config.toml");
        let mut doc: toml::Value = toml::from_str("[embeddings]\nthreads=0\n").unwrap();
        set_model_dir(&mut doc, Path::new(&model)).unwrap();
        fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
        let first = with_model(d.path(), |runtime| {
            Ok(runtime as *mut dyn Embedder as *mut () as usize)
        })
        .unwrap();
        let threads = ProjectConfig::load(d.path())
            .unwrap()
            .embeddings
            .effective_threads();
        doc["embeddings"]["threads"] = toml::Value::Integer(threads as i64);
        fs::write(&path, toml::to_string(&doc).unwrap()).unwrap();
        let second = with_model(d.path(), |runtime| {
            Ok(runtime as *mut dyn Embedder as *mut () as usize)
        })
        .unwrap();
        assert_eq!(
            first, second,
            "auto and its resolved count should reuse a session"
        );
    }
    #[test]
    fn setup_config_creates_missing_tables_and_preserves_other_settings() {
        let mut config: toml::Value = toml::from_str("[graph]\nmax_nodes=123\n").unwrap();
        set_model_dir(&mut config, Path::new("model")).unwrap();
        assert_eq!(config["graph"]["max_nodes"].as_integer(), Some(123));
        assert_eq!(config["embeddings"]["model_dir"].as_str(), Some("model"));
        set_model_dir(&mut config, Path::new("model")).unwrap();
        let cfg = toml::to_string(&config).unwrap();
        assert_eq!(cfg.matches("model_dir").count(), 1);
    }
    #[test]
    fn model_checksum_and_missing_files_fail_explicitly() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("model");
        let a = Artifact {
            file: "model".into(),
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        assert!(checksum(&p, &a).is_err());
        fs::write(&p, b"abc").unwrap();
        checksum(&p, &a).unwrap();
        fs::write(&p, b"abd").unwrap();
        assert!(
            checksum(&p, &a)
                .unwrap_err()
                .to_string()
                .contains("checksum")
        );
        fs::write(&p, b"too long").unwrap();
        assert!(checksum(&p, &a).unwrap_err().to_string().contains("size"));
    }
}
