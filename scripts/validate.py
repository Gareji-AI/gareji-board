"""Validate Gareji Board's schemas, fixtures, references, links, and public hygiene."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator


ROOT = Path(__file__).resolve().parents[1]


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def validate_json() -> None:
    for path in sorted(ROOT.rglob("*.json")):
        load_json(path)

    schema = load_json(ROOT / "schemas" / "progress-checkpoint-v0.schema.json")
    example = load_json(ROOT / "examples" / "progress-checkpoint.json")
    Draft202012Validator.check_schema(schema)
    Draft202012Validator(schema).validate(example)


def require_refs(values: list[str], valid: set[str], label: str) -> None:
    missing = sorted(set(values) - valid)
    if missing:
        raise ValueError(f"{label} references missing IDs: {', '.join(missing)}")


def validate_demo_refs() -> None:
    demo = load_json(ROOT / "examples" / "demo-board.json")
    workspaces = {item["id"] for item in demo["knowledge_workspaces"]}
    adapters = {item["id"] for item in demo["knowledge_adapter_catalog"]}
    projects = {item["id"] for item in demo["projects"]}
    sources = {item["id"] for item in demo["context_sources"]}
    execution_workspaces = {item["id"] for item in demo["execution_workspaces"]}
    agents = {item["id"] for item in demo["agents"]}
    skills = {item["id"] for item in demo["skill_catalog"]}
    work_items = {item["id"] for item in demo["work_items"]}

    for workspace in demo["knowledge_workspaces"]:
        require_refs([workspace["adapter"]], adapters, f"workspace {workspace['id']}")

    for connection in demo["knowledge_connections"]:
        require_refs([connection["project_id"]], projects, f"connection {connection['id']}")
        require_refs([connection["workspace_id"]], workspaces, f"connection {connection['id']}")

    for source in demo["context_sources"]:
        require_refs([source["workspace_id"]], workspaces, f"source {source['id']}")

    for workspace in demo["execution_workspaces"]:
        require_refs([workspace["project_id"]], projects, f"execution workspace {workspace['id']}")

    for agent in demo["agents"]:
        require_refs(agent.get("skills", []), skills, f"agent {agent['id']}")

    for project in demo["projects"]:
        require_refs(project.get("execution_workspace_ids", []), execution_workspaces, f"project {project['id']}")
        require_refs(project.get("context_source_ids", []), sources, f"project {project['id']}")

    for item in demo["work_items"]:
        require_refs([item["project_id"]], projects, f"work item {item['id']}")
        require_refs([item["agent_id"]], agents, f"work item {item['id']}")
        require_refs(item.get("context_source_ids", []), sources, f"work item {item['id']}")

    for run in demo["runs"]:
        require_refs([run["work_item_id"]], work_items, f"run {run['id']}")


def validate_markdown_links() -> None:
    link_pattern = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
    failures: list[str] = []
    for markdown in sorted(ROOT.rglob("*.md")):
        text = markdown.read_text(encoding="utf-8")
        for raw_target in link_pattern.findall(text):
            target = raw_target.strip().strip("<>").split("#", 1)[0]
            if not target or "://" in target or target.startswith("mailto:"):
                continue
            if not (markdown.parent / target).resolve().exists():
                failures.append(f"{markdown.relative_to(ROOT)} -> {raw_target}")
    if failures:
        raise ValueError("broken Markdown links:\n" + "\n".join(failures))


def validate_public_hygiene() -> None:
    denied = {
        "private inspiration name": re.compile(r"multica", re.IGNORECASE),
        "personal Windows path": re.compile(r"[A-Za-z]:[\\/]Users[\\/][^\\/\s]+"),
    }
    failures: list[str] = []
    paths = [*ROOT.rglob("*.md"), *ROOT.rglob("*.json"), *ROOT.rglob("*.yaml"), *ROOT.rglob("*.yml")]
    for path in sorted(set(paths)):
        if ".git" in path.parts:
            continue
        text = path.read_text(encoding="utf-8")
        for label, pattern in denied.items():
            if pattern.search(text):
                failures.append(f"{path.relative_to(ROOT)} contains {label}")
    if failures:
        raise ValueError("public hygiene failures:\n" + "\n".join(failures))


def main() -> int:
    checks = [validate_json, validate_demo_refs, validate_markdown_links, validate_public_hygiene]
    try:
        for check in checks:
            check()
            print(f"ok: {check.__name__}")
    except (OSError, KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"validation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
