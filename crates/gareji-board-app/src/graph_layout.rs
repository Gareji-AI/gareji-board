use std::collections::{HashMap, HashSet, VecDeque};

use gareji_board_domain::ControlGraphRevision;

/// Deterministic left-to-right projection of one Control graph.
///
/// Reachable nodes use their shortest distance from any entry. Nodes that are
/// not yet connected are kept visible in one final draft column.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlGraphLayout {
    columns: Vec<Vec<String>>,
    reachable_node_ids: HashSet<String>,
    position_by_node_id: HashMap<String, GraphNodePosition>,
}

#[cfg(test)]
const COLUMN_STEP: usize = 154;
#[cfg(test)]
const NODE_WIDTH: usize = 126;
#[cfg(test)]
const DEPTH_LABEL_HEIGHT: usize = 19;
#[cfg(test)]
const NODE_HEIGHT: usize = 68;
#[cfg(test)]
const ROW_STEP: usize = 75;

/// Pixel coordinate owned by the visual editor rather than execution semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasPoint {
    pub x: f64,
    pub y: f64,
}

const FREEFORM_COLUMN_STEP: f64 = 196.0;
const FREEFORM_ROW_STEP: f64 = 118.0;
const FREEFORM_LEFT: f64 = 34.0;
const FREEFORM_TOP: f64 = 58.0;
const FREEFORM_NODE_WIDTH: f64 = 164.0;
const FREEFORM_NODE_HEIGHT: f64 = 84.0;
const FREEFORM_MIN_WIDTH: f64 = 960.0;
const FREEFORM_MIN_HEIGHT: f64 = 520.0;
const FREEFORM_MARGIN: f64 = 80.0;

/// Stable column and row occupied by one laid-out Control node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphNodePosition {
    pub column: usize,
    pub row: usize,
}

impl ControlGraphLayout {
    /// Compute one stable layout while preserving source node order per column.
    #[must_use]
    pub fn new(graph: &ControlGraphRevision) -> Self {
        let node_ids = graph
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        let entry_node_ids = graph
            .entries
            .iter()
            .map(|entry| entry.node_id.clone())
            .collect::<Vec<_>>();
        let routes = graph
            .routes
            .iter()
            .map(|route| {
                (
                    route.source_node_id.clone(),
                    route.destination_node_id.clone(),
                )
            })
            .collect::<Vec<_>>();
        Self::from_topology(&node_ids, &entry_node_ids, &routes)
    }

    /// Compute the same projection for another Board-owned graph vocabulary.
    #[must_use]
    pub fn from_topology(
        node_ids: &[String],
        entry_node_ids: &[String],
        routes: &[(String, String)],
    ) -> Self {
        let mut depth_by_node = HashMap::<&str, usize>::new();
        let mut queue = VecDeque::new();
        for entry_node_id in entry_node_ids {
            if depth_by_node.insert(entry_node_id.as_str(), 0).is_none() {
                queue.push_back(entry_node_id.as_str());
            }
        }

        while let Some(source_node_id) = queue.pop_front() {
            let source_depth = depth_by_node[source_node_id];
            for destination_node_id in routes
                .iter()
                .filter(|(source, _)| source == source_node_id)
                .map(|(_, destination)| destination.as_str())
            {
                if depth_by_node.contains_key(destination_node_id) {
                    continue;
                }
                depth_by_node.insert(destination_node_id, source_depth + 1);
                queue.push_back(destination_node_id);
            }
        }

        let reachable_column_count = depth_by_node
            .values()
            .copied()
            .max()
            .map_or(0, |maximum_depth| maximum_depth + 1);
        let has_unreachable_nodes = node_ids
            .iter()
            .any(|node_id| !depth_by_node.contains_key(node_id.as_str()));
        let mut columns =
            vec![Vec::new(); reachable_column_count + usize::from(has_unreachable_nodes)];
        for node_id in node_ids {
            let column_index = depth_by_node
                .get(node_id.as_str())
                .copied()
                .unwrap_or(reachable_column_count);
            columns[column_index].push(node_id.clone());
        }

        let position_by_node_id = columns
            .iter()
            .enumerate()
            .flat_map(|(column, node_ids)| {
                node_ids
                    .iter()
                    .enumerate()
                    .map(move |(row, node_id)| (node_id.clone(), GraphNodePosition { column, row }))
            })
            .collect();

        Self {
            columns,
            reachable_node_ids: depth_by_node
                .keys()
                .map(|node_id| (*node_id).to_owned())
                .collect(),
            position_by_node_id,
        }
    }

    /// Read the stable left-to-right node columns.
    #[cfg(test)]
    #[must_use]
    pub fn columns(&self) -> &[Vec<String>] {
        &self.columns
    }

    /// Return whether one node is connected to at least one fixed entry.
    #[must_use]
    pub fn is_reachable(&self, node_id: &str) -> bool {
        self.reachable_node_ids.contains(node_id)
    }

    /// Return the stable logical position of one node.
    #[must_use]
    pub fn node_position(&self, node_id: &str) -> Option<GraphNodePosition> {
        self.position_by_node_id.get(node_id).copied()
    }

    /// Return the automatic pixel coordinate used until a person moves a node.
    #[must_use]
    pub fn canvas_point(&self, node_id: &str) -> Option<CanvasPoint> {
        let position = self.node_position(node_id)?;
        let column = u32::try_from(position.column).unwrap_or(u32::MAX);
        let row = u32::try_from(position.row).unwrap_or(u32::MAX);
        Some(CanvasPoint {
            x: FREEFORM_LEFT + f64::from(column) * FREEFORM_COLUMN_STEP,
            y: FREEFORM_TOP + f64::from(row) * FREEFORM_ROW_STEP,
        })
    }

    /// Return canvas dimensions that contain automatic and user-positioned nodes.
    #[must_use]
    pub fn freeform_stage_dimensions(
        &self,
        positioned_nodes: &HashMap<String, CanvasPoint>,
    ) -> (f64, f64) {
        let mut maximum_x = FREEFORM_MIN_WIDTH;
        let mut maximum_y = FREEFORM_MIN_HEIGHT;
        for node_id in self.position_by_node_id.keys() {
            let point = positioned_nodes
                .get(node_id)
                .copied()
                .or_else(|| self.canvas_point(node_id))
                .unwrap_or_default();
            maximum_x = maximum_x.max(point.x + FREEFORM_NODE_WIDTH + FREEFORM_MARGIN);
            maximum_y = maximum_y.max(point.y + FREEFORM_NODE_HEIGHT + FREEFORM_MARGIN);
        }
        (maximum_x.ceil(), maximum_y.ceil())
    }

    /// Find the first visible grid slot that does not overlap another node.
    #[must_use]
    pub fn next_open_canvas_point(
        &self,
        positioned_nodes: &HashMap<String, CanvasPoint>,
    ) -> CanvasPoint {
        let occupied = self
            .position_by_node_id
            .keys()
            .filter_map(|node_id| {
                positioned_nodes
                    .get(node_id)
                    .copied()
                    .or_else(|| self.canvas_point(node_id))
            })
            .collect::<Vec<_>>();
        for row in 0_u32..100 {
            for column in 0_u32..5 {
                let candidate = CanvasPoint {
                    x: FREEFORM_LEFT + f64::from(column) * FREEFORM_COLUMN_STEP,
                    y: FREEFORM_TOP + f64::from(row) * FREEFORM_ROW_STEP,
                };
                let is_open = occupied.iter().all(|point| {
                    (point.x - candidate.x).abs() >= FREEFORM_NODE_WIDTH + 20.0
                        || (point.y - candidate.y).abs() >= FREEFORM_NODE_HEIGHT + 20.0
                });
                if is_open {
                    return candidate;
                }
            }
        }
        CanvasPoint {
            x: FREEFORM_LEFT,
            y: FREEFORM_TOP + 100.0 * FREEFORM_ROW_STEP,
        }
    }

    /// Build one SVG path between freely positioned node cards.
    #[cfg(test)]
    #[must_use]
    pub fn freeform_route_path(
        &self,
        source_node_id: &str,
        destination_node_id: &str,
        positioned_nodes: &HashMap<String, CanvasPoint>,
    ) -> Option<String> {
        self.freeform_branch_route_path(
            source_node_id,
            destination_node_id,
            positioned_nodes,
            FREEFORM_NODE_HEIGHT / 2.0,
            FREEFORM_NODE_HEIGHT / 2.0,
        )
    }

    /// Build one SVG path between explicit source and destination port offsets.
    #[must_use]
    pub fn freeform_branch_route_path(
        &self,
        source_node_id: &str,
        destination_node_id: &str,
        positioned_nodes: &HashMap<String, CanvasPoint>,
        source_port_y: f64,
        destination_port_y: f64,
    ) -> Option<String> {
        let source = positioned_nodes
            .get(source_node_id)
            .copied()
            .or_else(|| self.canvas_point(source_node_id))?;
        let destination = positioned_nodes
            .get(destination_node_id)
            .copied()
            .or_else(|| self.canvas_point(destination_node_id))?;
        let source_x = source.x + FREEFORM_NODE_WIDTH;
        let source_y = source.y + source_port_y;
        let destination_x = destination.x;
        let destination_y = destination.y + destination_port_y;
        let horizontal_distance = (destination_x - source_x).abs();
        let control_distance = (horizontal_distance * 0.5).max(52.0);
        Some(format!(
            "M {source_x:.1} {source_y:.1} C {:.1} {source_y:.1}, {:.1} {destination_y:.1}, {destination_x:.1} {destination_y:.1}",
            source_x + control_distance,
            destination_x - control_distance,
        ))
    }

    /// Return the pixel dimensions required by the graph stage.
    #[cfg(test)]
    #[must_use]
    pub fn stage_dimensions(&self) -> (usize, usize) {
        let width = self
            .columns
            .len()
            .saturating_sub(1)
            .saturating_mul(COLUMN_STEP)
            .saturating_add(NODE_WIDTH);
        let maximum_rows = self.columns.iter().map(Vec::len).max().unwrap_or(1);
        let height = DEPTH_LABEL_HEIGHT
            .saturating_add(maximum_rows.saturating_mul(ROW_STEP))
            .max(DEPTH_LABEL_HEIGHT + NODE_HEIGHT);
        (width, height)
    }

    /// Build one SVG path connecting the visible edges of two node cards.
    #[cfg(test)]
    #[must_use]
    pub fn route_path(&self, source_node_id: &str, destination_node_id: &str) -> Option<String> {
        let source = self.node_position(source_node_id)?;
        let destination = self.node_position(destination_node_id)?;
        let source_x = source.column.saturating_mul(COLUMN_STEP) + NODE_WIDTH;
        let source_y = DEPTH_LABEL_HEIGHT + source.row.saturating_mul(ROW_STEP) + NODE_HEIGHT / 2;
        let destination_x = destination.column.saturating_mul(COLUMN_STEP);
        let destination_y =
            DEPTH_LABEL_HEIGHT + destination.row.saturating_mul(ROW_STEP) + NODE_HEIGHT / 2;
        if destination_x > source_x {
            let middle_x = usize::midpoint(source_x, destination_x);
            Some(format!(
                "M {source_x} {source_y} C {middle_x} {source_y}, {middle_x} {destination_y}, {destination_x} {destination_y}"
            ))
        } else {
            let bend_y = source_y.max(destination_y).saturating_add(28);
            Some(format!(
                "M {source_x} {source_y} C {} {bend_y}, {} {bend_y}, {destination_x} {destination_y}",
                source_x.saturating_add(18),
                destination_x.saturating_sub(18),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gareji_board_domain::{
        ControlGraphRevision, ControlNode, ControlNodeKind, ControlRoute, ControlSignal,
        GraphAnchor, GraphEntry,
    };

    use super::{CanvasPoint, ControlGraphLayout, GraphNodePosition};

    #[test]
    fn lays_out_branches_cycles_and_unconnected_draft_nodes_stably() {
        let graph = ControlGraphRevision {
            graph_id: "layout".to_owned(),
            revision_id: "v1".to_owned(),
            entries: vec![GraphEntry {
                id: "standard".to_owned(),
                node_id: "start".to_owned(),
            }],
            nodes: ["start", "left", "right", "finish", "draft"]
                .into_iter()
                .map(|id| ControlNode {
                    id: id.to_owned(),
                    kind: ControlNodeKind::AgentLoop {
                        agent_profile_id: "implementer".to_owned(),
                    },
                })
                .collect(),
            routes: vec![
                route("start-left", "start", "left", ControlSignal::Succeeded),
                route("start-right", "start", "right", ControlSignal::Failed),
                route("left-finish", "left", "finish", ControlSignal::Succeeded),
                route("finish-left", "finish", "left", ControlSignal::Failed),
            ],
            anchors: vec![GraphAnchor {
                id: "fixed".to_owned(),
                description: "The test layout remains grounded".to_owned(),
            }],
        };

        let layout = ControlGraphLayout::new(&graph);

        assert_eq!(
            layout.columns(),
            &[
                vec!["start".to_owned()],
                vec!["left".to_owned(), "right".to_owned()],
                vec!["finish".to_owned()],
                vec!["draft".to_owned()],
            ]
        );
        assert!(layout.is_reachable("finish"));
        assert!(!layout.is_reachable("draft"));
        assert_eq!(
            layout.node_position("right"),
            Some(GraphNodePosition { column: 1, row: 1 })
        );
        assert_eq!(layout.stage_dimensions(), (588, 169));
        assert_eq!(
            layout.route_path("start", "right").as_deref(),
            Some("M 126 53 C 140 53, 140 128, 154 128")
        );
        assert_eq!(
            layout.route_path("finish", "left").as_deref(),
            Some("M 434 53 C 452 81, 136 81, 154 53")
        );

        let mut positioned_nodes = HashMap::new();
        positioned_nodes.insert("start".to_owned(), CanvasPoint { x: 50.0, y: 80.0 });
        positioned_nodes.insert("left".to_owned(), CanvasPoint { x: 420.0, y: 220.0 });
        assert_eq!(
            layout.freeform_stage_dimensions(&positioned_nodes),
            (960.0, 520.0)
        );
        assert_eq!(
            layout
                .freeform_route_path("start", "left", &positioned_nodes)
                .as_deref(),
            Some("M 214.0 122.0 C 317.0 122.0, 317.0 262.0, 420.0 262.0")
        );
        assert_eq!(
            layout
                .freeform_branch_route_path("start", "left", &positioned_nodes, 56.0, 42.0)
                .as_deref(),
            Some("M 214.0 136.0 C 317.0 136.0, 317.0 262.0, 420.0 262.0")
        );
        assert_eq!(
            layout.next_open_canvas_point(&positioned_nodes),
            CanvasPoint { x: 818.0, y: 58.0 }
        );
    }

    fn route(
        id: &str,
        source_node_id: &str,
        destination_node_id: &str,
        signal: ControlSignal,
    ) -> ControlRoute {
        ControlRoute {
            id: id.to_owned(),
            source_node_id: source_node_id.to_owned(),
            destination_node_id: destination_node_id.to_owned(),
            signal,
        }
    }
}
