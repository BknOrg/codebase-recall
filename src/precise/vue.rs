//! Vue's official TypeScript request bridge; source documents remain unmodified.
use super::{
    backend::{self, Launcher},
    client::LspClient,
};
use crate::config::ProjectConfig;
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub struct Dependencies {
    pub sdk: PathBuf,
    pub plugin: PathBuf,
}
pub fn dependencies(
    project: &Path,
    launcher: &Launcher,
    cfg: &ProjectConfig,
) -> Result<Dependencies> {
    let mut roots: Vec<_> = project
        .ancestors()
        .map(|p| p.join("node_modules"))
        .collect();
    if let Some(parent) = launcher.program.parent() {
        roots.push(parent.join("node_modules"));
        roots.push(parent.to_path_buf());
        if let Some(parent) = parent.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    let sdk=cfg.precise.vue.tsdk.as_ref().map(|p|project.join(p)).or_else(||roots.iter().map(|p|p.join("typescript/lib")).find(|p|p.join("typescript.js").is_file())).context("dependency_missing: Vue TypeScript SDK not found; install typescript or set [precise.vue] tsdk to typescript/lib")?;
    let plugin=cfg.precise.vue.plugin_path.as_ref().map(|p|project.join(p)).or_else(||roots.iter().flat_map(|p|[p.join("@vue/typescript-plugin"),p.join("@vue/language-server/node_modules/@vue/typescript-plugin")]).find(|p|p.join("package.json").is_file())).context("dependency_missing: @vue/typescript-plugin not found; install it or set [precise.vue] plugin_path")?;
    ensure!(
        sdk.join("typescript.js").is_file() && sdk.join("tsserver.js").is_file(),
        "dependency_missing: Vue tsdk must contain typescript.js and tsserver.js"
    );
    ensure!(
        plugin.join("package.json").is_file(),
        "dependency_missing: Vue plugin_path must name the @vue/typescript-plugin package directory"
    );
    Ok(Dependencies {
        sdk: crate::c_family::absolute(&sdk),
        plugin: crate::c_family::absolute(&plugin),
    })
}
pub fn companion(
    project: &Path,
    launcher: &Launcher,
    cfg: &ProjectConfig,
    deadline: Option<Instant>,
    timeout: Duration,
) -> Result<LspClient> {
    let dependencies = dependencies(project, launcher, cfg)?;
    let ts = backend::BACKENDS
        .iter()
        .find(|b| b.lang_group == "typescript")
        .unwrap()
        .locate()
        .map_err(anyhow::Error::msg)?;
    let mut client = LspClient::start(&ts, project)?;
    if let Some(deadline) = deadline {
        client.set_deadline(deadline);
    }
    client.initialize_with(project,timeout,json!({"tsserver":{"path":dependencies.sdk.join("tsserver.js")},"plugins":[{"name":"@vue/typescript-plugin","location":dependencies.plugin,"languages":["vue"]}]}))?;
    ensure!(
        client.supports_command("typescript.tsserverRequest"),
        "unsupported_capability: Vue companion must support typescript.tsserverRequest; use a compatible typescript-language-server via CODE_RCL_LSP_TYPESCRIPT"
    );
    Ok(client)
}
