//! Turning the flat list of layer records into the tree they describe.
//!
//! Photoshop stores a group as two records: a "bounding section divider" that marks where
//! the group's members end (it comes *first* in the file, because the file runs bottom to
//! top), then the members, then the group's own record (an open or closed folder) carrying
//! its name, opacity and blend mode. Everything here is iterative and bounded, so a file
//! of a thousand unclosed groups costs a thousand pushes and then an error.
use super::document::{Divider, LayerRecord};
use super::error::PsdError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// An ordinary layer: an index into the record list.
    Layer(usize),
    /// A group: its own record's index and its members, bottom first.
    Group { record: usize, children: Vec<Node> },
}

impl Node {
    pub fn record(&self) -> usize {
        match self {
            Self::Layer(index) | Self::Group { record: index, .. } => *index,
        }
    }
}

/// `max_layers` counts layers and groups (a divider record that closes a group is the
/// group, not a second layer); `max_depth` is the deepest nesting allowed, with a layer
/// at the root at depth 1.
pub fn build_tree(
    records: &[LayerRecord],
    max_layers: usize,
    max_depth: usize,
) -> Result<Vec<Node>, PsdError> {
    let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
    let mut counted = 0_usize;
    for (index, record) in records.iter().enumerate() {
        match record.divider {
            Some(Divider::BoundingSection) => {
                if stack.len() >= max_depth {
                    return Err(PsdError::Limit {
                        what: "group nesting depth",
                        value: stack.len() as u64 + 1,
                        limit: max_depth as u64,
                    });
                }
                stack.push(Vec::new());
            }
            Some(Divider::OpenFolder) | Some(Divider::ClosedFolder) => {
                if stack.len() < 2 {
                    return Err(PsdError::Malformed {
                        what: "a group record with no matching group start",
                    });
                }
                let children = stack.pop().unwrap_or_default();
                counted += 1;
                if counted > max_layers {
                    return Err(PsdError::Limit {
                        what: "layers",
                        value: counted as u64,
                        limit: max_layers as u64,
                    });
                }
                if let Some(parent) = stack.last_mut() {
                    parent.push(Node::Group {
                        record: index,
                        children,
                    });
                }
            }
            _ => {
                counted += 1;
                if counted > max_layers {
                    return Err(PsdError::Limit {
                        what: "layers",
                        value: counted as u64,
                        limit: max_layers as u64,
                    });
                }
                if let Some(parent) = stack.last_mut() {
                    parent.push(Node::Layer(index));
                }
            }
        }
    }
    if stack.len() != 1 {
        return Err(PsdError::Malformed {
            what: "a group that is never closed",
        });
    }
    Ok(stack.pop().unwrap_or_default())
}
