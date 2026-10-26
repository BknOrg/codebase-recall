//! Shared, preflighted installation/removal of agent resources.
use super::{EMBEDDED_SKILL, REFERENCES, clean_path, get_executable_command, get_home_dir, integ};
use crate::cli::SetupArgs;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const AGENTS: [&str; 3] = ["codex", "claude", "antigravity"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Skill,
    JsonMcp,
    TomlMcp,
    Instructions,
    GitHook,
    ClaudeHook,
}
#[derive(Clone)]
struct Resource {
    path: PathBuf,
    kind: Kind,
    owners: BTreeSet<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Installed {
    kind: Kind,
    owners: BTreeSet<String>,
    files: Vec<String>,
    #[serde(default)]
    legacy_shared: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    resources: BTreeMap<String, Installed>,
}
struct Mutation {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    executable: bool,
}

fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                ensure!(out.pop(), "invalid path");
            }
            std::path::Component::CurDir => (),
            part => out.push(part.as_os_str()),
        }
    }
    Ok(PathBuf::from(clean_path(&out)))
}
fn key(path: &Path) -> String {
    let s = clean_path(path).replace('\\', "/");
    if cfg!(windows) { s.to_lowercase() } else { s }
}
fn no_symlinks(path: &Path, boundary: &Path) -> Result<()> {
    let mut p = Some(path);
    while let Some(current) = p {
        if let Ok(meta) = fs::symlink_metadata(current) {
            ensure!(
                !meta.is_symlink(),
                "refusing symlink resource: {}",
                current.display()
            );
        }
        if current == boundary {
            break;
        }
        p = current.parent();
    }
    Ok(())
}
fn definitions(
    base: &Path,
    codex: &Path,
    global: bool,
    custom: Option<&Path>,
) -> Result<Vec<Resource>> {
    let mut resources: BTreeMap<String, Resource> = BTreeMap::new();
    let mut add = |agent: &str, path: PathBuf, kind| -> Result<()> {
        let resource = resources.entry(key(&path)).or_insert_with(|| Resource {
            path,
            kind,
            owners: BTreeSet::new(),
        });
        ensure!(
            resource.kind == kind,
            "resource paths overlap: {}",
            resource.path.display()
        );
        resource.owners.insert(agent.to_string());
        Ok(())
    };
    for agent in AGENTS {
        let skill = if let Some(p) = custom {
            absolute(p)?
        } else if !global {
            base.join(if agent == "claude" {
                ".claude/skills/code-rcl"
            } else {
                ".agents/skills/code-rcl"
            })
        } else {
            base.join(match agent {
                "codex" => ".agents/skills/code-rcl",
                "claude" => ".claude/skills/code-rcl",
                _ => ".gemini/config/skills/code-rcl",
            })
        };
        add(agent, skill, Kind::Skill)?;
        let (mcp, kind) = match agent {
            "codex" => (codex.join("config.toml"), Kind::TomlMcp),
            "claude" => (
                base.join(if global { ".claude.json" } else { ".mcp.json" }),
                Kind::JsonMcp,
            ),
            _ => (
                base.join(if global {
                    ".gemini/config/mcp_config.json"
                } else {
                    ".agents/mcp_config.json"
                }),
                Kind::JsonMcp,
            ),
        };
        add(agent, mcp, kind)?;
        let instructions = if agent == "codex" {
            if global {
                codex.join("AGENTS.md")
            } else {
                base.join("AGENTS.md")
            }
        } else if agent == "claude" {
            base.join(if global {
                ".claude/CLAUDE.md"
            } else {
                "CLAUDE.md"
            })
        } else {
            base.join(if global {
                ".gemini/GEMINI.md"
            } else {
                "GEMINI.md"
            })
        };
        add(agent, instructions, Kind::Instructions)?;
        if agent == "antigravity" && !global {
            add(agent, base.join("AGENTS.md"), Kind::Instructions)?;
        }
        if agent == "claude" {
            add(agent, base.join(".claude/settings.json"), Kind::ClaudeHook)?;
        }
    }
    Ok(resources.into_values().collect())
}
fn json_mcp(
    bytes: Option<&[u8]>,
    exe: &str,
    project: Option<&Path>,
    remove: bool,
) -> Result<Vec<u8>> {
    let mut root: Value = match bytes {
        Some(b) if !b.iter().all(u8::is_ascii_whitespace) => {
            serde_json::from_slice(b).context("not valid JSON; refusing to overwrite it")?
        }
        _ => json!({}),
    };
    ensure!(root.is_object(), "MCP config must contain a JSON object");
    if let Some(servers) = root.get("mcpServers") {
        ensure!(servers.is_object(), "mcpServers must be an object");
    }
    if remove {
        if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
            servers.remove("code-rcl");
        }
    } else {
        let servers = root
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        let server = servers
            .as_object_mut()
            .unwrap()
            .entry("code-rcl")
            .or_insert_with(|| json!({}));
        ensure!(server.is_object(), "code-rcl MCP entry must be an object");
        server["command"] = json!(exe);
        server["args"] = if let Some(project) = project {
            json!(["mcp", "--project", clean_path(project)])
        } else {
            json!(["mcp"])
        };
    }
    if let Some(bytes) = bytes
        && serde_json::from_slice::<Value>(bytes).ok().as_ref() == Some(&root)
    {
        return Ok(bytes.to_vec());
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?).into_bytes())
}
fn toml_mcp(
    bytes: Option<&[u8]>,
    exe: &str,
    project: Option<&Path>,
    remove: bool,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes.unwrap_or_default())?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .context("not valid TOML; refusing to overwrite it")?;
    if let Some(servers) = doc.get("mcp_servers") {
        ensure!(servers.is_table_like(), "mcp_servers must be a table");
    }
    if remove {
        if let Some(servers) = doc
            .get_mut("mcp_servers")
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            servers.remove("code-rcl");
        }
    } else {
        if doc.get("mcp_servers").is_none() {
            doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        if doc["mcp_servers"].get("code-rcl").is_none() {
            doc["mcp_servers"]["code-rcl"] = if doc["mcp_servers"].as_value().is_some() {
                toml_edit::value(toml_edit::InlineTable::new())
            } else {
                toml_edit::Item::Table(toml_edit::Table::new())
            };
        }
        ensure!(
            doc["mcp_servers"]["code-rcl"].is_table_like(),
            "code-rcl MCP entry must be a table"
        );
        doc["mcp_servers"]["code-rcl"]["command"] = toml_edit::value(exe);
        let mut args = toml_edit::Array::new();
        args.push("mcp");
        if let Some(p) = project {
            args.push("--project");
            args.push(clean_path(p));
        }
        doc["mcp_servers"]["code-rcl"]["args"] = toml_edit::value(args);
    }
    let result = doc.to_string();
    let parsed: toml::Value = toml::from_str(&result)?;
    ensure!(
        remove
            || parsed
                .get("mcp_servers")
                .and_then(|v| v.get("code-rcl"))
                .and_then(|v| v.get("command"))
                .and_then(toml::Value::as_str)
                == Some(exe),
        "MCP registration did not serialize correctly"
    );
    Ok(result.into_bytes())
}
fn registered(resource: &Resource) -> Result<bool> {
    let Some(bytes) = read(&resource.path)? else {
        return Ok(false);
    };
    if resource.kind == Kind::JsonMcp {
        let root: Value = serde_json::from_slice(&bytes)?;
        Ok(root
            .get("mcpServers")
            .and_then(|v| v.get("code-rcl"))
            .is_some())
    } else {
        let root: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
        Ok(root
            .get("mcp_servers")
            .and_then(|v| v.get("code-rcl"))
            .is_some())
    }
}
fn selected(kind: Kind, args: &SetupArgs, remove: bool) -> bool {
    match kind {
        Kind::Skill => !args.mcp_only,
        Kind::JsonMcp | Kind::TomlMcp => !args.skill_only,
        _ if remove => !args.skill_only && !args.mcp_only,
        Kind::Instructions => args.instructions,
        Kind::GitHook => args.git_hook,
        Kind::ClaudeHook => args.claude_hook,
    }
}
fn bundle() -> Vec<(String, String)> {
    std::iter::once(("SKILL.md".into(), super::bundled_text(EMBEDDED_SKILL)))
        .chain(
            REFERENCES
                .iter()
                .map(|(n, s)| (format!("references/{n}"), super::bundled_text(s))),
        )
        .collect()
}
fn stage(path: &Path, kind: Kind, remove: bool, exe: &str) -> Result<Option<Vec<u8>>> {
    let dir = tempfile::tempdir()?;
    let scratch = dir.path().join("resource");
    if let Some(bytes) = read(path)? {
        fs::write(&scratch, bytes)?;
    }
    match (kind, remove) {
        (Kind::Instructions, false) => {
            integ::upsert_marked_block(
                &scratch,
                integ::MD_BEGIN,
                integ::MD_END,
                integ::INSTRUCTIONS_BODY,
            )?;
        }
        (Kind::Instructions, true) => {
            integ::remove_marked_block(&scratch, integ::MD_BEGIN, integ::MD_END)?;
        }
        (Kind::GitHook, false) => {
            integ::install_git_hook(&scratch, exe)?;
        }
        (Kind::GitHook, true) => {
            integ::remove_git_hook(&scratch)?;
        }
        (Kind::ClaudeHook, false) => {
            integ::install_claude_hook(&scratch, exe)?;
        }
        (Kind::ClaudeHook, true) => {
            integ::remove_claude_hook(&scratch)?;
        }
        _ => unreachable!(),
    }
    read(&scratch)
}
fn mutation(path: PathBuf, after: Option<Vec<u8>>, executable: bool) -> Result<Mutation> {
    let before = read(&path)?;
    Ok(Mutation {
        path,
        before,
        after,
        executable,
    })
}
fn apply(m: &Mutation) -> Result<()> {
    ensure!(
        read(&m.path)? == m.before,
        "{} changed after preflight; retry",
        m.path.display()
    );
    if m.before == m.after {
        return Ok(());
    }
    if let Some(bytes) = &m.after {
        let parent = m.path.parent().context("missing parent directory")?;
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(bytes)?;
        if let Ok(meta) = fs::metadata(&m.path) {
            temp.as_file().set_permissions(meta.permissions())?;
        }
        #[cfg(unix)]
        if m.executable && m.before.is_none() {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        #[cfg(not(unix))]
        let _ = m.executable;
        temp.as_file().sync_all()?;
        temp.persist(&m.path)?;
    } else if m.before.is_some() {
        fs::remove_file(&m.path)?;
    }
    Ok(())
}

pub(super) fn run(args: &SetupArgs, remove: bool) -> Result<()> {
    let target = args.target.to_lowercase();
    let target = if target == "gemini" {
        "antigravity"
    } else {
        target.as_str()
    };
    let targets: BTreeSet<String> = AGENTS
        .into_iter()
        .filter(|a| target == "all" || *a == target)
        .map(str::to_string)
        .collect();
    ensure!(!targets.is_empty(), "unknown agent target");
    ensure!(
        !(args.global && args.workspace),
        "--global conflicts with --workspace"
    );
    ensure!(
        !(args.skill_only && args.mcp_only),
        "--skill-only conflicts with --mcp-only"
    );
    ensure!(
        !(args.mcp_only && args.skill_dir.is_some()),
        "--skill-dir conflicts with --mcp-only"
    );
    let base = if args.global {
        absolute(&get_home_dir().context("cannot determine home for --global")?)?
    } else {
        absolute(&std::env::current_dir()?)?
    };
    let codex = if args.global {
        absolute(
            &std::env::var_os("CODEX_HOME")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join(".codex")),
        )?
    } else {
        base.join(".codex")
    };
    let mut resources = definitions(&base, &codex, args.global, args.skill_dir.as_deref())?;
    if args.global && remove && args.skill_dir.is_none() {
        resources.push(Resource {
            path: codex.join("skills/code-rcl"),
            kind: Kind::Skill,
            owners: BTreeSet::from(["codex".into()]),
        });
    }
    if !args.global && (args.git_hook || remove) {
        // Not being in git must not prevent skill/MCP-only uninstall.
        match integ::post_commit_hook_path(&base) {
            Ok(path) => resources.push(Resource {
                path: absolute(&path)?,
                kind: Kind::GitHook,
                owners: AGENTS.map(str::to_string).into(),
            }),
            Err(e) if args.git_hook => return Err(e),
            Err(_) => (),
        }
    }
    let mut unique: BTreeMap<String, Resource> = BTreeMap::new();
    for resource in resources {
        if let Some(existing) = unique.get_mut(&key(&resource.path)) {
            ensure!(
                existing.kind == resource.kind,
                "resource paths overlap: {}",
                resource.path.display()
            );
            existing.owners.extend(resource.owners);
        } else {
            unique.insert(key(&resource.path), resource);
        }
    }
    let resources: Vec<_> = unique.into_values().collect();
    let state_path = base.join(".code-rcl/agent-integrations.json");
    no_symlinks(&state_path, &base)?;
    let state_before = read(&state_path)?;
    let mut state: State = if let Some(bytes) = &state_before {
        serde_json::from_slice(bytes).context("invalid agent integration manifest")?
    } else {
        State {
            version: 1,
            resources: BTreeMap::new(),
        }
    };
    ensure!(
        state.version == 1,
        "unsupported agent integration manifest version"
    );
    let exe = get_executable_command();
    let mut changes = Vec::new();
    let mut empty_dirs = Vec::new();
    for resource in &resources {
        let owners: BTreeSet<_> = resource.owners.intersection(&targets).cloned().collect();
        if owners.is_empty() || !selected(resource.kind, args, remove) {
            continue;
        }
        no_symlinks(&resource.path, &base)?;
        let id = key(&resource.path);
        let previous = state.resources.get(&id).cloned();
        if let Some(previous) = &previous {
            ensure!(
                previous.kind == resource.kind,
                "manifest resource kind mismatch: {}",
                resource.path.display()
            );
        }
        let exists = if resource.kind == Kind::Skill {
            resource.path.join("SKILL.md").exists()
                || (previous.is_some() && resource.path.exists())
        } else {
            resource.path.exists()
        };
        let legacy_present = if matches!(resource.kind, Kind::Instructions | Kind::GitHook) {
            read(&resource.path)?.is_some_and(|b| {
                String::from_utf8_lossy(&b).contains(if resource.kind == Kind::GitHook {
                    integ::SH_BEGIN
                } else {
                    integ::MD_BEGIN
                })
            })
        } else {
            exists
        };
        let mut installed = previous.clone().unwrap_or_else(|| Installed {
            kind: resource.kind,
            owners: BTreeSet::new(),
            files: Vec::new(),
            legacy_shared: legacy_present && resource.owners.len() > 1 && target != "all",
        });
        if remove {
            for owner in &targets {
                installed.owners.remove(owner);
            }
            // A manually registered, unselected agent may also consume the shared skill.
            if resource.owners.len() > 1 {
                for other in resource.owners.difference(&targets) {
                    for mcp in resources.iter().filter(|r| {
                        matches!(r.kind, Kind::JsonMcp | Kind::TomlMcp) && r.owners.contains(other)
                    }) {
                        match registered(mcp) {
                            Ok(true) => {
                                installed.owners.insert(other.clone());
                            }
                            Err(_) => installed.legacy_shared = true,
                            Ok(false) => (),
                        }
                    }
                }
            }
            if !installed.owners.is_empty() || (installed.legacy_shared && target != "all") {
                println!(
                    "Retained shared/legacy resource: {} (other or unknown owners)",
                    resource.path.display()
                );
                state.resources.insert(id, installed);
                continue;
            }
            state.resources.remove(&id);
            if !exists {
                continue;
            }
        } else {
            installed.owners.extend(owners);
            if target == "all" {
                installed.legacy_shared = false;
            }
            installed.files = if resource.kind == Kind::Skill {
                bundle().into_iter().map(|(name, _)| name).collect()
            } else {
                vec![]
            };
            state.resources.insert(id, installed);
        }
        let boundary = if let Some(custom) = &args.skill_dir {
            if resource.kind == Kind::Skill {
                absolute(custom)?
            } else {
                base.clone()
            }
        } else {
            base.clone()
        };
        match resource.kind {
            Kind::Skill => {
                if remove && previous.is_none() {
                    let text = fs::read_to_string(resource.path.join("SKILL.md"))?;
                    ensure!(
                        text.lines().any(|l| l.trim() == "name: code-rcl"),
                        "unrecognized legacy skill: {}",
                        resource.path.display()
                    );
                }
                for (name, text) in bundle() {
                    let path = resource.path.join(name);
                    no_symlinks(&path, &boundary)?;
                    changes.push(mutation(
                        path,
                        if remove {
                            None
                        } else {
                            Some(text.as_bytes().to_vec())
                        },
                        false,
                    )?);
                }
                if remove {
                    empty_dirs.push(resource.path.join("references"));
                    empty_dirs.push(resource.path.clone());
                }
            }
            Kind::JsonMcp | Kind::TomlMcp => {
                let before = read(&resource.path)?;
                let project = (!args.global).then_some(base.as_path());
                let after = if resource.kind == Kind::JsonMcp {
                    json_mcp(before.as_deref(), &exe, project, remove)
                } else {
                    toml_mcp(before.as_deref(), &exe, project, remove)
                }
                .with_context(|| format!("preflight {}", resource.path.display()))?;
                changes.push(mutation(resource.path.clone(), Some(after), false)?);
            }
            kind => {
                let after = stage(&resource.path, kind, remove, &exe)
                    .with_context(|| format!("preflight {}", resource.path.display()))?;
                changes.push(mutation(
                    resource.path.clone(),
                    after,
                    kind == Kind::GitHook,
                )?);
            }
        }
    }
    if remove {
        let known: BTreeSet<_> = resources.iter().map(|r| key(&r.path)).collect();
        for (path, installed) in &state.resources {
            if !known.contains(path)
                && selected(installed.kind, args, true)
                && !installed.owners.is_disjoint(&targets)
            {
                println!(
                    "Retained resource outside current target paths: {path}; repeat --skill-dir or the original agent home setting to select it"
                );
            }
        }
    }
    let after = if state.resources.is_empty() {
        None
    } else {
        Some(serde_json::to_vec_pretty(&state)?)
    };
    changes.push(Mutation {
        path: state_path,
        before: state_before,
        after,
        executable: false,
    });
    let mut completed = 0;
    for change in &changes {
        apply(change).with_context(|| format!("{} after {completed} completed file changes; partial changes may remain; fix the error and rerun", change.path.display()))?;
        if change.before != change.after {
            completed += 1;
            println!(
                "{}: {}",
                if change.after.is_some() {
                    "Updated"
                } else {
                    "Removed"
                },
                change.path.display()
            );
        }
    }
    for dir in empty_dirs {
        if dir.is_dir() && fs::read_dir(&dir)?.next().is_none() {
            fs::remove_dir(dir)?;
        }
    }
    if completed == 0 {
        println!("already up to date; nothing to change");
    }
    println!(
        "code-rcl {} completed successfully",
        if remove { "uninstall" } else { "setup" }
    );
    if !remove && targets.contains("codex") {
        println!(
            "Codex: project config requires a trusted workspace; reconnect the client to reload MCP."
        );
    }
    if args.global && args.git_hook {
        println!("Git hooks are per-repository; run workspace setup --git-hook inside the repo.");
    }
    Ok(())
}
