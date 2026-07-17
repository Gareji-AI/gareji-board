//! Bounded local Interface for Board-owned Work item assessments.

use std::path::Path;

use gareji_board_store::{SqliteBoardStore, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BOARD_BRIDGE_PROTOCOL_VERSION: &str = "gareji.board-bridge.v0";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoardBridgeRequest {
    pub protocol_version: String,
    pub request_id: String,
    #[serde(flatten)]
    pub operation: BoardBridgeOperation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum BoardBridgeOperation {
    AssessActiveWork {
        project_id: String,
        work_item_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BoardBridgeResponse {
    Ok {
        protocol_version: String,
        request_id: String,
        result: Value,
    },
    Error {
        protocol_version: String,
        request_id: String,
        error: BoardBridgeError,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BoardBridgeError {
    pub code: BoardBridgeErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardBridgeErrorCode {
    InvalidRequest,
    ProjectNotFound,
    WorkItemNotFound,
    InternalError,
}

/// Deep Module that keeps Board state decisions behind one transport operation.
pub struct BoardBridge {
    store: SqliteBoardStore,
}

impl BoardBridge {
    /// Open Board-owned state from its local `SQLite` file.
    ///
    /// # Errors
    ///
    /// Returns a bounded storage error when Board state cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Ok(Self {
            store: SqliteBoardStore::open(path)?,
        })
    }

    /// Compose the Interface with a local-substitutable store for tests.
    #[must_use]
    pub fn new(store: SqliteBoardStore) -> Self {
        Self { store }
    }

    #[must_use]
    pub fn handle(&self, request: BoardBridgeRequest) -> BoardBridgeResponse {
        let request_id = request.request_id;
        if request.protocol_version != BOARD_BRIDGE_PROTOCOL_VERSION {
            return error_response(
                request_id,
                BoardBridgeErrorCode::InvalidRequest,
                "unsupported Board bridge protocol",
            );
        }
        if request_id.is_empty() || request_id.chars().count() > 128 {
            return error_response(
                request_id,
                BoardBridgeErrorCode::InvalidRequest,
                "invalid request identity",
            );
        }

        let result = match request.operation {
            BoardBridgeOperation::AssessActiveWork {
                project_id,
                work_item_id,
            } => self
                .store
                .assess_active_work(&project_id, &work_item_id)
                .and_then(|assessment| {
                    serde_json::to_value(assessment)
                        .map_err(|_| StoreError::CorruptState("assessment serialization failed"))
                }),
        };
        match result {
            Ok(result) => BoardBridgeResponse::Ok {
                protocol_version: BOARD_BRIDGE_PROTOCOL_VERSION.to_owned(),
                request_id,
                result,
            },
            Err(error) => map_store_error(request_id, &error),
        }
    }
}

fn map_store_error(request_id: String, error: &StoreError) -> BoardBridgeResponse {
    match error {
        StoreError::InvalidRequest => error_response(
            request_id,
            BoardBridgeErrorCode::InvalidRequest,
            "invalid active-work assessment request",
        ),
        StoreError::ProjectNotFound => error_response(
            request_id,
            BoardBridgeErrorCode::ProjectNotFound,
            "Board project was not found",
        ),
        StoreError::WorkItemNotFound => error_response(
            request_id,
            BoardBridgeErrorCode::WorkItemNotFound,
            "Work item was not found in the requested project",
        ),
        StoreError::CreateDirectory(_) | StoreError::Sqlite(_) | StoreError::CorruptState(_) => {
            error_response(
                request_id,
                BoardBridgeErrorCode::InternalError,
                "local Board operation failed",
            )
        }
    }
}

fn error_response(
    request_id: String,
    code: BoardBridgeErrorCode,
    message: &str,
) -> BoardBridgeResponse {
    BoardBridgeResponse::Error {
        protocol_version: BOARD_BRIDGE_PROTOCOL_VERSION.to_owned(),
        request_id,
        error: BoardBridgeError {
            code,
            message: message.chars().take(256).collect(),
        },
    }
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{ActiveWorkAssessment, ActiveWorkEligibility, WorkItemState};

    use super::*;

    fn request(project_id: &str, work_item_id: &str) -> BoardBridgeRequest {
        BoardBridgeRequest {
            protocol_version: BOARD_BRIDGE_PROTOCOL_VERSION.to_owned(),
            request_id: "request-1".to_owned(),
            operation: BoardBridgeOperation::AssessActiveWork {
                project_id: project_id.to_owned(),
                work_item_id: work_item_id.to_owned(),
            },
        }
    }

    fn seeded_bridge() -> BoardBridge {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        BoardBridge::new(store)
    }

    #[test]
    fn bridge_returns_the_board_owned_assessment() {
        let response = seeded_bridge().handle(request("gareji-core", "CORE-2"));
        let BoardBridgeResponse::Ok { result, .. } = response else {
            panic!("expected an assessment");
        };
        let assessment: ActiveWorkAssessment = serde_json::from_value(result).unwrap();
        assert_eq!(assessment.state, WorkItemState::Todo);
        assert_eq!(assessment.eligibility, ActiveWorkEligibility::Eligible);
    }

    #[test]
    fn bridge_does_not_reveal_an_item_from_another_project() {
        let response = seeded_bridge().handle(request("gareji-board", "CORE-2"));
        assert!(matches!(
            response,
            BoardBridgeResponse::Error {
                error: BoardBridgeError {
                    code: BoardBridgeErrorCode::WorkItemNotFound,
                    ..
                },
                ..
            }
        ));
    }
}
