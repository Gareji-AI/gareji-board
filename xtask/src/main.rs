use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value;
use walkdir::WalkDir;

fn main() -> Result<()> {
    let command = env::args().nth(1).unwrap_or_else(|| "validate".to_owned());
    if command != "validate" {
        bail!("unknown xtask command `{command}`; expected `validate`");
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask must live directly below the repository root")?
        .to_path_buf();
    run_check("json", || validate_json(&root))?;
    run_check("demo_refs", || validate_demo_refs(&root))?;
    run_check("markdown_links", || validate_markdown_links(&root))?;
    run_check("public_hygiene", || validate_public_hygiene(&root))?;
    Ok(())
}

fn run_check(name: &str, check: impl FnOnce() -> Result<()>) -> Result<()> {
    check().with_context(|| format!("validation failed: {name}"))?;
    println!("ok: {name}");
    Ok(())
}

fn repository_files(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            name != ".git" && name != "target"
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
}

fn load_json(path: &Path) -> Result<Value> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    serde_json::from_str(&contents).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn validate_json(root: &Path) -> Result<()> {
    for path in repository_files(root).filter(|path| has_extension(path, "json")) {
        load_json(&path)?;
    }

    let schema = load_json(&root.join("schemas/progress-checkpoint-v0.schema.json"))?;
    let example = load_json(&root.join("examples/progress-checkpoint.json"))?;
    jsonschema::meta::validate(&schema)
        .map_err(|error| anyhow::anyhow!("invalid progress-checkpoint meta-schema: {error}"))?;
    jsonschema::draft202012::validate(&schema, &example).map_err(|error| {
        anyhow::anyhow!("progress-checkpoint example does not match the schema: {error}")
    })?;
    Ok(())
}

fn validate_demo_refs(root: &Path) -> Result<()> {
    let demo = load_json(&root.join("examples/demo-board.json"))?;
    let workspaces = ids(&demo, "knowledge_workspaces")?;
    let adapters = ids(&demo, "knowledge_adapter_catalog")?;
    let projects = ids(&demo, "projects")?;
    let sources = ids(&demo, "context_sources")?;
    let execution_workspaces = ids(&demo, "execution_workspaces")?;
    let agents = ids(&demo, "agents")?;
    let skills = ids(&demo, "skill_catalog")?;
    let work_items = ids(&demo, "work_items")?;

    for item in array(&demo, "knowledge_workspaces")? {
        require_refs(
            &strings(item, "adapter", false)?,
            &adapters,
            &label(item, "workspace")?,
        )?;
    }
    for item in array(&demo, "knowledge_connections")? {
        require_refs(
            &strings(item, "project_id", false)?,
            &projects,
            &label(item, "connection")?,
        )?;
        require_refs(
            &strings(item, "workspace_id", false)?,
            &workspaces,
            &label(item, "connection")?,
        )?;
    }
    for item in array(&demo, "context_sources")? {
        require_refs(
            &strings(item, "workspace_id", false)?,
            &workspaces,
            &label(item, "source")?,
        )?;
    }
    for item in array(&demo, "execution_workspaces")? {
        require_refs(
            &strings(item, "project_id", false)?,
            &projects,
            &label(item, "execution workspace")?,
        )?;
    }
    for item in array(&demo, "agents")? {
        require_refs(
            &strings(item, "skills", true)?,
            &skills,
            &label(item, "agent")?,
        )?;
    }
    for item in array(&demo, "projects")? {
        require_refs(
            &strings(item, "execution_workspace_ids", true)?,
            &execution_workspaces,
            &label(item, "project")?,
        )?;
        require_refs(
            &strings(item, "context_source_ids", true)?,
            &sources,
            &label(item, "project")?,
        )?;
    }
    for item in array(&demo, "work_items")? {
        require_refs(
            &strings(item, "project_id", false)?,
            &projects,
            &label(item, "work item")?,
        )?;
        require_refs(
            &strings(item, "agent_id", false)?,
            &agents,
            &label(item, "work item")?,
        )?;
        require_refs(
            &strings(item, "context_source_ids", true)?,
            &sources,
            &label(item, "work item")?,
        )?;
    }
    for item in array(&demo, "runs")? {
        require_refs(
            &strings(item, "work_item_id", false)?,
            &work_items,
            &label(item, "run")?,
        )?;
    }
    validate_demo_agent_files(root, &demo)?;
    Ok(())
}

fn validate_demo_agent_files(root: &Path, demo: &Value) -> Result<()> {
    let fixtures = array(demo, "execution_workspaces")?
        .iter()
        .map(|workspace| {
            let fixture = string_field(workspace, "fixture")?;
            let label = label(workspace, "execution workspace")?;
            require_relative_path(fixture, &label)?;
            let path = root.join(fixture);
            if !path.is_dir() {
                bail!("{label} fixture is not a directory: {fixture}");
            }
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;

    for skill in array(demo, "skill_catalog")? {
        let source = string_field(skill, "source")?;
        let skill_label = label(skill, "skill")?;
        require_relative_path(source, &skill_label)?;
        require_contained_file(root, &root.join(source).join("SKILL.md"), &skill_label)?;
    }
    for agent in array(demo, "agents")? {
        let instruction = string_field(agent, "instructions")?;
        let agent_label = label(agent, "agent")?;
        require_relative_path(instruction, &agent_label)?;
        let resolves = fixtures.iter().any(|fixture| {
            require_contained_file(fixture, &fixture.join(instruction), &agent_label).is_ok()
        });
        if !resolves {
            bail!(
                "{agent_label} instruction does not resolve in a demo Execution workspace: {instruction}"
            );
        }
    }
    Ok(())
}

fn require_relative_path(value: &str, label: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
        || !path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        bail!("{label} has an unsafe relative path: {value}");
    }
    Ok(())
}

fn require_contained_file(root: &Path, candidate: &Path, label: &str) -> Result<()> {
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("could not inspect {label} root: {}", root.display()))?;
    let canonical_candidate = fs::canonicalize(candidate)
        .with_context(|| format!("missing {label} file: {}", candidate.display()))?;
    if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
        bail!(
            "{label} file escapes its root or is not regular: {}",
            candidate.display()
        );
    }
    Ok(())
}

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .with_context(|| format!("`{field}` must be an array"))
}

fn ids(value: &Value, field: &str) -> Result<BTreeSet<String>> {
    array(value, field)?
        .iter()
        .map(|item| string_field(item, "id").map(str::to_owned))
        .collect()
}

fn strings(value: &Value, field: &str, optional: bool) -> Result<Vec<String>> {
    let Some(raw) = value.get(field) else {
        return if optional {
            Ok(Vec::new())
        } else {
            bail!("missing required field `{field}`")
        };
    };
    if let Some(single) = raw.as_str() {
        return Ok(vec![single.to_owned()]);
    }
    raw.as_array()
        .with_context(|| format!("`{field}` must be a string or array"))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .with_context(|| format!("`{field}` contains a non-string value"))
        })
        .collect()
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("missing string field `{field}`"))
}

fn label(value: &Value, kind: &str) -> Result<String> {
    Ok(format!("{kind} {}", string_field(value, "id")?))
}

fn require_refs(values: &[String], valid: &BTreeSet<String>, label: &str) -> Result<()> {
    let missing: Vec<_> = values
        .iter()
        .filter(|value| !valid.contains(*value))
        .cloned()
        .collect();
    if !missing.is_empty() {
        bail!("{label} references missing IDs: {}", missing.join(", "));
    }
    Ok(())
}

fn validate_markdown_links(root: &Path) -> Result<()> {
    let link_pattern = Regex::new(r"\[[^\]]+\]\(([^)]+)\)")?;
    let mut failures = Vec::new();
    for markdown in repository_files(root).filter(|path| has_extension(path, "md")) {
        let contents = fs::read_to_string(&markdown)?;
        for captures in link_pattern.captures_iter(&contents) {
            let raw = captures.get(1).context("missing link capture")?.as_str();
            let target = raw
                .trim()
                .trim_matches(['<', '>'])
                .split('#')
                .next()
                .unwrap_or_default();
            if target.is_empty() || target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            let resolved = markdown.parent().unwrap_or(root).join(target);
            if !resolved.exists() {
                failures.push(format!(
                    "{} -> {raw}",
                    markdown.strip_prefix(root).unwrap_or(&markdown).display()
                ));
            }
        }
    }
    if !failures.is_empty() {
        bail!("broken Markdown links:\n{}", failures.join("\n"));
    }
    Ok(())
}

fn validate_public_hygiene(root: &Path) -> Result<()> {
    let denied = [
        ("private inspiration name", Regex::new("(?i)multica")?),
        (
            "personal Windows path",
            Regex::new(r"[A-Za-z]:[\\/]Users[\\/][^\\/\s]+")?,
        ),
    ];
    let mut failures = Vec::new();
    for path in repository_files(root).filter(|path| {
        ["md", "json", "yaml", "yml"]
            .iter()
            .any(|extension| has_extension(path, extension))
    }) {
        let contents = fs::read_to_string(&path)?;
        for (label, pattern) in &denied {
            if pattern.is_match(&contents) {
                failures.push(format!(
                    "{} contains {label}",
                    path.strip_prefix(root).unwrap_or(&path).display()
                ));
            }
        }
    }
    if !failures.is_empty() {
        bail!("public hygiene failures:\n{}", failures.join("\n"));
    }
    Ok(())
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .is_some_and(|extension| extension == expected)
}
