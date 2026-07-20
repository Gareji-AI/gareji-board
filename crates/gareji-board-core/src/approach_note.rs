use std::collections::HashMap;
use std::fs;
use std::path::Path;

use gareji_board_domain::{
    ApproachNoteManifest, ApproachNoteValidationError, ApproachRisk, NoteSocketKind,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

const DEFAULT_MAX_NOTE_BYTES: u64 = 256 * 1024;

/// Bounded local Markdown reader for project-independent Approach Notes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkdownApproachNoteReader {
    max_note_bytes: u64,
}

impl Default for MarkdownApproachNoteReader {
    fn default() -> Self {
        Self {
            max_note_bytes: DEFAULT_MAX_NOTE_BYTES,
        }
    }
}

impl MarkdownApproachNoteReader {
    /// Read, canonicalize, fingerprint, and validate one absolute Markdown path.
    ///
    /// The note body remains in the knowledge file. The returned manifest holds
    /// only bounded metadata, its canonical locator, and a content fingerprint.
    ///
    /// # Errors
    ///
    /// Returns a bounded path, I/O, frontmatter, or manifest validation error.
    pub fn read(self, path: &Path) -> Result<ApproachNoteManifest, ApproachNoteReadError> {
        if !path.is_absolute() {
            return Err(ApproachNoteReadError::PathNotAbsolute);
        }
        let canonical_path = fs::canonicalize(path).map_err(ApproachNoteReadError::Open)?;
        if !is_markdown_path(&canonical_path) {
            return Err(ApproachNoteReadError::NotMarkdown);
        }
        let metadata = fs::metadata(&canonical_path).map_err(ApproachNoteReadError::Open)?;
        if !metadata.is_file() {
            return Err(ApproachNoteReadError::NotFile);
        }
        if metadata.len() > self.max_note_bytes {
            return Err(ApproachNoteReadError::TooLarge {
                actual_bytes: metadata.len(),
                max_bytes: self.max_note_bytes,
            });
        }
        let content = fs::read(&canonical_path).map_err(ApproachNoteReadError::Read)?;
        let text = std::str::from_utf8(&content).map_err(|_| ApproachNoteReadError::NotUtf8)?;
        let (frontmatter, body) = split_frontmatter(text)?;
        if body.trim().is_empty() {
            return Err(ApproachNoteReadError::MissingBody);
        }
        let fields = parse_fields(frontmatter)?;
        let manifest = ApproachNoteManifest {
            approach_id: required_field(&fields, "id")?.to_owned(),
            title: required_field(&fields, "title")?.to_owned(),
            absolute_path: canonical_path.display().to_string(),
            fingerprint: format!("{:x}", Sha256::digest(&content)),
            required_capabilities: parse_list(optional_field(&fields, "required_capabilities"))?,
            inputs: parse_sockets(required_field(&fields, "inputs")?)?,
            outputs: parse_sockets(required_field(&fields, "outputs")?)?,
            risk: parse_risk(required_field(&fields, "risk")?)?,
        };
        manifest
            .validate()
            .map_err(ApproachNoteReadError::InvalidManifest)?;
        Ok(manifest)
    }
}

/// Expected failure while reading a local Markdown Approach Note.
#[derive(Debug, Error)]
pub enum ApproachNoteReadError {
    #[error("Approach Note path must be absolute")]
    PathNotAbsolute,
    #[error("Approach Note must use a .md or .markdown extension")]
    NotMarkdown,
    #[error("Approach Note path must identify a regular file")]
    NotFile,
    #[error("Approach Note is larger than {max_bytes} bytes ({actual_bytes} bytes)")]
    TooLarge { actual_bytes: u64, max_bytes: u64 },
    #[error("Approach Note could not be opened: {0}")]
    Open(std::io::Error),
    #[error("Approach Note could not be read: {0}")]
    Read(std::io::Error),
    #[error("Approach Note must be UTF-8")]
    NotUtf8,
    #[error("Approach Note must begin with bounded YAML-like frontmatter")]
    MissingFrontmatter,
    #[error("Approach Note frontmatter contains an invalid line")]
    InvalidFrontmatterLine,
    #[error("Approach Note frontmatter repeats field {field}")]
    DuplicateField { field: String },
    #[error("Approach Note frontmatter is missing field {field}")]
    MissingField { field: &'static str },
    #[error("Approach Note list field is invalid")]
    InvalidList,
    #[error("Approach Note declares unknown socket {socket}")]
    UnknownSocket { socket: String },
    #[error("Approach Note declares unknown risk {risk}")]
    UnknownRisk { risk: String },
    #[error("Approach Note must include a non-empty Markdown body")]
    MissingBody,
    #[error(transparent)]
    InvalidManifest(ApproachNoteValidationError),
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
        })
}

fn split_frontmatter(text: &str) -> Result<(&str, &str), ApproachNoteReadError> {
    let normalized = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = normalized
        .strip_prefix("---\n")
        .or_else(|| normalized.strip_prefix("---\r\n"))
        .ok_or(ApproachNoteReadError::MissingFrontmatter)?;
    if let Some((frontmatter, body)) = rest.split_once("\n---\n") {
        return Ok((frontmatter, body));
    }
    if let Some((frontmatter, body)) = rest.split_once("\r\n---\r\n") {
        return Ok((frontmatter, body));
    }
    Err(ApproachNoteReadError::MissingFrontmatter)
}

fn parse_fields(frontmatter: &str) -> Result<HashMap<String, String>, ApproachNoteReadError> {
    let mut fields = HashMap::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(ApproachNoteReadError::InvalidFrontmatterLine);
        };
        let key = key.trim();
        let value = trim_quotes(value.trim());
        if key.is_empty() || value.is_empty() {
            return Err(ApproachNoteReadError::InvalidFrontmatterLine);
        }
        if fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(ApproachNoteReadError::DuplicateField {
                field: key.to_owned(),
            });
        }
    }
    Ok(fields)
}

fn trim_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn required_field<'a>(
    fields: &'a HashMap<String, String>,
    field: &'static str,
) -> Result<&'a str, ApproachNoteReadError> {
    fields
        .get(field)
        .map(String::as_str)
        .ok_or(ApproachNoteReadError::MissingField { field })
}

fn optional_field<'a>(fields: &'a HashMap<String, String>, field: &str) -> &'a str {
    fields.get(field).map_or("[]", String::as_str)
}

fn parse_list(value: &str) -> Result<Vec<String>, ApproachNoteReadError> {
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or(ApproachNoteReadError::InvalidList)?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|item| {
            let item = trim_quotes(item.trim());
            if item.is_empty() {
                Err(ApproachNoteReadError::InvalidList)
            } else {
                Ok(item.to_owned())
            }
        })
        .collect()
}

fn parse_sockets(value: &str) -> Result<Vec<NoteSocketKind>, ApproachNoteReadError> {
    parse_list(value)?
        .into_iter()
        .map(|socket| {
            let kind = match socket.as_str() {
                "context" => NoteSocketKind::Context,
                "work_item" => NoteSocketKind::WorkItem,
                "evidence" => NoteSocketKind::Evidence,
                "artifact" => NoteSocketKind::Artifact,
                "signal" => NoteSocketKind::Signal,
                "approval" => NoteSocketKind::Approval,
                _ => return Err(ApproachNoteReadError::UnknownSocket { socket }),
            };
            Ok(kind)
        })
        .collect()
}

fn parse_risk(value: &str) -> Result<ApproachRisk, ApproachNoteReadError> {
    match value {
        "low" => Ok(ApproachRisk::Low),
        "moderate" => Ok(ApproachRisk::Moderate),
        "high" => Ok(ApproachRisk::High),
        "critical" => Ok(ApproachRisk::Critical),
        _ => Err(ApproachNoteReadError::UnknownRisk {
            risk: value.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use gareji_board_domain::{ApproachRisk, NoteSocketKind};

    use super::{ApproachNoteReadError, MarkdownApproachNoteReader};

    #[test]
    fn reads_absolute_markdown_note_and_pins_its_content() {
        let directory = temporary_directory("valid");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("evidence-first.md");
        fs::write(
            &path,
            "---\nid: evidence-first\ntitle: Evidence first\nrisk: low\nrequired_capabilities: [research, testing]\ninputs: [context, work_item]\noutputs: [evidence]\n---\n\n# Evidence first\n\nGather independent evidence.\n",
        )
        .unwrap();

        let manifest = MarkdownApproachNoteReader::default().read(&path).unwrap();

        assert_eq!(manifest.approach_id, "evidence-first");
        assert_eq!(manifest.risk, ApproachRisk::Low);
        assert_eq!(
            manifest.inputs,
            vec![NoteSocketKind::Context, NoteSocketKind::WorkItem]
        );
        assert_eq!(manifest.outputs, vec![NoteSocketKind::Evidence]);
        assert_eq!(manifest.fingerprint.len(), 64);
        assert_eq!(
            manifest.absolute_path,
            fs::canonicalize(&path).unwrap().display().to_string()
        );

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn rejects_relative_note_inputs() {
        assert!(matches!(
            MarkdownApproachNoteReader::default().read(std::path::Path::new("approach.md")),
            Err(ApproachNoteReadError::PathNotAbsolute)
        ));
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "gareji-board-approach-note-{}-{label}-{nonce}",
            std::process::id()
        ))
    }
}
