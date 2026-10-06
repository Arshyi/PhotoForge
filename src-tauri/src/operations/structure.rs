//! Editing the layer tree: the structural half of every operation.
//!
//! These functions change a `LayerDocument` in place and report failure with an
//! error rather than by quietly returning the document unchanged, which is what
//! the TypeScript tree functions do. A caller that asked for something impossible
//! should be told so.
//!
//! None of them is atomic on its own: a function that fails may have changed the
//! document before it noticed. That is deliberate, because they are only ever
//! called by the transaction engine on a private copy that it throws away on any
//! error. Making each one all-or-nothing would add a clone to every step to guard
//! against a caller that does not exist.
use super::support::{Clock, IdSource};
use crate::error::AppError;
use crate::layers::{
    BlendMode, Layer, LayerContent, LayerDocument, LayerKind, LayerMetadata, LayerTransform,
};
use std::collections::BTreeMap;

/// A layer with the defaults every new layer starts from.
pub fn new_layer(id: &str, name: &str, content: LayerContent, now: &str) -> Layer {
    Layer {
        id: id.to_string(),
        name: name.to_string(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata {
            created_at: now.to_string(),
            modified_at: now.to_string(),
            custom: BTreeMap::new(),
        },
        raw: None,
        origin: None,
        content,
    }
}

/// Every identifier in a layer's subtree, the layer first.
pub fn subtree_ids(layer: &Layer) -> Vec<String> {
    let mut ids = Vec::new();
    let mut stack = vec![layer];
    while let Some(current) = stack.pop() {
        ids.push(current.id.clone());
        stack.extend(current.children());
    }
    ids
}

fn not_found(id: &str) -> AppError {
    AppError::LayerNotFound(id.to_string())
}

/// Edits one layer in place and stamps its modification time.
pub fn update(
    document: &mut LayerDocument,
    id: &str,
    now: &str,
    edit: impl FnOnce(&mut Layer) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let layer = document.layer_mut(id).ok_or_else(|| not_found(id))?;
    edit(layer)?;
    layer.metadata.modified_at = now.to_string();
    Ok(())
}

/// Inserts a new layer and makes it the selected one.
pub fn insert_layer(
    document: &mut LayerDocument,
    layer: Layer,
    parent: Option<&str>,
    index: usize,
) -> Result<(), AppError> {
    let id = layer.id.clone();
    document.insert(layer, parent, index)?;
    document.active_layer_id = Some(id);
    Ok(())
}

/// Removes a layer and its subtree. The selection is cleared if it was inside.
pub fn remove_layer(document: &mut LayerDocument, id: &str) -> Result<(), AppError> {
    let removed_ids = subtree_ids(document.find(id).ok_or_else(|| not_found(id))?);
    document.remove(id)?;
    if document
        .active_layer_id
        .as_ref()
        .is_some_and(|active| removed_ids.contains(active))
    {
        document.active_layer_id = None;
    }
    Ok(())
}

/// Copies a layer and its whole subtree with new identifiers, placing the copy
/// directly above the original. Pixel buffers are shared: they are immutable, so
/// a duplicate costs no pixel memory until one of the copies is edited.
pub fn duplicate_layer(
    document: &mut LayerDocument,
    id: &str,
    ids: &mut dyn IdSource,
    clock: &dyn Clock,
) -> Result<String, AppError> {
    let original = document.find(id).ok_or_else(|| not_found(id))?.clone();
    let now = clock.now();
    let copy = with_new_ids(
        &original,
        Some(format!("{} copy", original.name)),
        ids,
        &now,
    );
    let path = document.path_to(id).ok_or_else(|| not_found(id))?;
    let (position, parent_path) = path.split_last().ok_or_else(|| not_found(id))?;
    let parent = parent_of(document, parent_path);
    let copy_id = copy.id.clone();
    document.insert(copy, parent.as_deref(), position + 1)?;
    document.active_layer_id = Some(copy_id.clone());
    Ok(copy_id)
}

fn with_new_ids(layer: &Layer, name: Option<String>, ids: &mut dyn IdSource, now: &str) -> Layer {
    let mut copy = layer.clone();
    copy.id = ids.next("l");
    if let Some(name) = name {
        copy.name = name;
    }
    if let LayerContent::Group { children, .. } = &mut copy.content {
        for child in children.iter_mut() {
            *child = with_new_ids(child, None, ids, now);
        }
    }
    copy.metadata.created_at = now.to_string();
    copy.metadata.modified_at = now.to_string();
    copy
}

/// The identifier of the layer at `path`, or `None` for the root stack.
fn parent_of(document: &LayerDocument, path: &[usize]) -> Option<String> {
    if path.is_empty() {
        None
    } else {
        document.layer_at(path).map(|layer| layer.id.clone())
    }
}

/// Wraps layers that share a parent in a new group, placed where the lowest of
/// them sat. The children keep the order they had in the stack, whatever order
/// the caller named them in.
pub fn group_layers(
    document: &mut LayerDocument,
    layer_ids: &[String],
    name: &str,
    ids: &mut dyn IdSource,
    clock: &dyn Clock,
) -> Result<String, AppError> {
    if layer_ids.is_empty() {
        return Err(AppError::InvalidLayerDocument(
            "grouping needs at least one layer".into(),
        ));
    }
    let mut paths = Vec::with_capacity(layer_ids.len());
    for id in layer_ids {
        paths.push(document.path_to(id).ok_or_else(|| not_found(id))?);
    }
    let parent_path = paths[0][..paths[0].len() - 1].to_vec();
    if paths
        .iter()
        .any(|path| path.len() != parent_path.len() + 1 || path[..path.len() - 1] != parent_path)
    {
        return Err(AppError::InvalidLayerDocument(
            "only layers that share a parent can be grouped together".into(),
        ));
    }
    let mut indices: Vec<usize> = paths.iter().map(|path| path[path.len() - 1]).collect();
    indices.sort_unstable();
    if indices.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(AppError::InvalidLayerDocument(
            "a layer was named twice".into(),
        ));
    }
    let mut ordered = Vec::with_capacity(indices.len());
    for index in &indices {
        let mut path = parent_path.clone();
        path.push(*index);
        ordered.push(
            document
                .layer_at(&path)
                .ok_or_else(|| not_found(&layer_ids[0]))?
                .clone(),
        );
    }
    let parent = parent_of(document, &parent_path);
    for id in layer_ids {
        document.remove(id)?;
    }
    let group_id = ids.next("l");
    let group = new_layer(
        &group_id,
        name,
        LayerContent::Group {
            children: ordered,
            isolated: true,
        },
        &clock.now(),
    );
    document.insert(group, parent.as_deref(), indices[0])?;
    document.active_layer_id = Some(group_id.clone());
    Ok(group_id)
}

/// Replaces a group with its children, in place and in order.
pub fn ungroup_layer(document: &mut LayerDocument, id: &str) -> Result<(), AppError> {
    let group = document.find(id).ok_or_else(|| not_found(id))?;
    if group.kind() != LayerKind::Group {
        return Err(AppError::InvalidLayerDocument(format!(
            "{} is not a group",
            group.name
        )));
    }
    let children: Vec<Layer> = group.children().to_vec();
    let path = document.path_to(id).ok_or_else(|| not_found(id))?;
    let (position, parent_path) = path.split_last().ok_or_else(|| not_found(id))?;
    let parent = parent_of(document, parent_path);
    let position = *position;
    // The selection may be a child of the group. While the group is out of the tree
    // that child does not exist, and every insertion validates the document, so
    // the selection is set aside and put back once the children are in.
    let previous = document.active_layer_id.take();
    document.remove(id)?;
    let selected = children.last().map(|child| child.id.clone());
    // The TypeScript behaviour: select the last child, or leave the selection alone.
    for (offset, child) in children.into_iter().enumerate() {
        document.insert(child, parent.as_deref(), position + offset)?;
    }
    document.active_layer_id = selected.or(previous);
    Ok(())
}

/// Whether a layer, or any layer it sits inside, is locked.
pub fn ensure_unlocked(document: &LayerDocument, id: &str) -> Result<(), AppError> {
    let path = document.path_to(id).ok_or_else(|| not_found(id))?;
    for end in 1..=path.len() {
        if let Some(layer) = document.layer_at(&path[..end]) {
            if layer.locked {
                return Err(AppError::LayerLocked(layer.name.clone()));
            }
        }
    }
    Ok(())
}

/// As `ensure_unlocked`, and no descendant of the layer is locked either.
pub fn ensure_subtree_unlocked(document: &LayerDocument, id: &str) -> Result<(), AppError> {
    ensure_unlocked(document, id)?;
    let layer = document.find(id).ok_or_else(|| not_found(id))?;
    let mut stack: Vec<&Layer> = layer.children().iter().collect();
    while let Some(current) = stack.pop() {
        if current.locked {
            return Err(AppError::LayerLocked(current.name.clone()));
        }
        stack.extend(current.children());
    }
    Ok(())
}

/// The transform a "reset" returns to: the identity, keeping the interpolation the
/// user chose for the layer.
pub fn reset_transform(layer: &mut Layer) {
    layer.transform = LayerTransform {
        interpolation: layer.transform.interpolation,
        ..LayerTransform::default()
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::support::{FixedClock, SequenceIds};

    fn pixel(id: &str) -> Layer {
        new_layer(
            id,
            id,
            LayerContent::Pixel {
                pixel_id: format!("px-{id}"),
                width: 16,
                height: 16,
            },
            "t0",
        )
    }

    fn document(ids: &[&str]) -> LayerDocument {
        let mut document = LayerDocument::new(16, 16);
        document.layers = ids.iter().map(|id| pixel(id)).collect();
        document
    }

    fn names(document: &LayerDocument) -> Vec<String> {
        document
            .layers
            .iter()
            .map(|layer| layer.id.clone())
            .collect()
    }

    #[test]
    fn inserting_selects_the_new_layer_and_refuses_a_duplicate_identifier() {
        let mut doc = document(&["a"]);
        insert_layer(&mut doc, pixel("b"), None, 1).unwrap();
        assert_eq!(doc.active_layer_id.as_deref(), Some("b"));
        assert!(insert_layer(&mut doc, pixel("b"), None, 0).is_err());
    }

    #[test]
    fn removing_a_group_clears_a_selection_inside_it() {
        let mut doc = document(&["a"]);
        let mut ids = SequenceIds::default();
        let clock = FixedClock("t1".into());
        let group = group_layers(&mut doc, &["a".into()], "G", &mut ids, &clock).unwrap();
        doc.active_layer_id = Some("a".into());
        remove_layer(&mut doc, &group).unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(doc.active_layer_id, None);
    }

    #[test]
    fn duplicating_gives_every_layer_in_the_subtree_a_new_identifier() {
        let mut doc = document(&["a", "b", "c"]);
        let mut ids = SequenceIds::default();
        let clock = FixedClock("t1".into());
        let group =
            group_layers(&mut doc, &["a".into(), "b".into()], "G", &mut ids, &clock).unwrap();
        let copy = duplicate_layer(&mut doc, &group, &mut ids, &clock).unwrap();
        assert_ne!(copy, group);
        assert_eq!(doc.layers.len(), 3, "the copy sits beside the original");
        assert_eq!(doc.layers[1].id, copy, "directly above it");
        assert_eq!(doc.layers[1].name, "G copy");
        let originals: Vec<String> = subtree_ids(&doc.layers[0]);
        let copies: Vec<String> = subtree_ids(&doc.layers[1]);
        assert_eq!(copies.len(), 3);
        assert!(copies.iter().all(|id| !originals.contains(id)));
        // Pixel buffers are shared, not copied.
        let first = doc
            .find(&copies[1])
            .and_then(|layer| layer.pixel_id().map(str::to_string));
        assert!(first.as_deref().is_some_and(|id| id.starts_with("px-")));
        doc.validate().unwrap();
    }

    #[test]
    fn grouping_keeps_stack_order_and_refuses_layers_with_different_parents() {
        let mut doc = document(&["a", "b", "c", "d"]);
        let mut ids = SequenceIds::default();
        let clock = FixedClock("t1".into());
        // Named out of order; the children keep the order they had.
        let group =
            group_layers(&mut doc, &["d".into(), "b".into()], "G", &mut ids, &clock).unwrap();
        assert_eq!(names(&doc), ["a", &group, "c"]);
        let children: Vec<&str> = doc
            .find(&group)
            .unwrap()
            .children()
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(children, ["b", "d"]);
        // "a" is at the root and "b" is inside the group.
        assert!(group_layers(&mut doc, &["a".into(), "b".into()], "H", &mut ids, &clock).is_err());
        assert!(group_layers(&mut doc, &[], "H", &mut ids, &clock).is_err());
        assert!(group_layers(&mut doc, &["a".into(), "a".into()], "H", &mut ids, &clock).is_err());
    }

    #[test]
    fn ungrouping_restores_the_children_in_place_and_selects_the_last() {
        let mut doc = document(&["a", "b", "c"]);
        let mut ids = SequenceIds::default();
        let clock = FixedClock("t1".into());
        let group =
            group_layers(&mut doc, &["b".into(), "c".into()], "G", &mut ids, &clock).unwrap();
        ungroup_layer(&mut doc, &group).unwrap();
        assert_eq!(names(&doc), ["a", "b", "c"]);
        assert_eq!(doc.active_layer_id.as_deref(), Some("c"));
        assert!(
            ungroup_layer(&mut doc, "a").is_err(),
            "a pixel layer is not a group"
        );
    }

    #[test]
    fn locks_are_checked_up_the_tree_and_down_it() {
        let mut doc = document(&["a", "b"]);
        let mut ids = SequenceIds::default();
        let clock = FixedClock("t1".into());
        let group =
            group_layers(&mut doc, &["a".into(), "b".into()], "G", &mut ids, &clock).unwrap();
        ensure_unlocked(&doc, "a").unwrap();
        doc.layer_mut(&group).unwrap().locked = true;
        // A layer inside a locked group is protected by it.
        assert!(matches!(
            ensure_unlocked(&doc, "a"),
            Err(AppError::LayerLocked(_))
        ));
        doc.layer_mut(&group).unwrap().locked = false;
        doc.layer_mut("b").unwrap().locked = true;
        ensure_unlocked(&doc, &group).unwrap();
        // ...but a locked child blocks anything that rewrites the whole subtree.
        assert!(ensure_subtree_unlocked(&doc, &group).is_err());
    }

    #[test]
    fn reset_transform_keeps_the_chosen_interpolation() {
        let mut layer = pixel("a");
        layer.transform.scale_x = 2.0;
        layer.transform.rotation_degrees = 30.0;
        layer.transform.interpolation = crate::layers::LayerInterpolation::Nearest;
        reset_transform(&mut layer);
        assert_eq!(layer.transform.scale_x, 1.0);
        assert_eq!(layer.transform.rotation_degrees, 0.0);
        assert_eq!(
            layer.transform.interpolation,
            crate::layers::LayerInterpolation::Nearest
        );
    }
}
