use crate::api::types::{Comment, CommentNode};

pub(crate) fn flatten_visible_comments(tree: &[CommentNode]) -> Vec<Comment> {
    fn walk(nodes: &[CommentNode], out: &mut Vec<Comment>) {
        for node in nodes {
            out.push(node.comment.clone());
            if !node.comment.collapsed {
                walk(&node.children, out);
            }
        }
    }

    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

pub(crate) fn apply_default_expansion(tree: &mut [CommentNode], visible_levels: usize) {
    let expand_depth_exclusive = visible_levels.saturating_sub(1);

    fn walk(nodes: &mut [CommentNode], expand_depth_exclusive: usize) {
        for node in nodes {
            if node.comment.depth < expand_depth_exclusive && !node.comment.kids.is_empty() {
                node.comment.collapsed = false;
            }
            if !node.children.is_empty() {
                walk(&mut node.children, expand_depth_exclusive);
            }
        }
    }

    walk(tree, expand_depth_exclusive);
}

pub(crate) fn set_collapse(tree: &mut [CommentNode], target: u64, collapsed: bool) -> Option<()> {
    for node in tree {
        if node.comment.id == target {
            node.comment.collapsed = collapsed;
            return Some(());
        }
        if set_collapse(&mut node.children, target, collapsed).is_some() {
            return Some(());
        }
    }
    None
}

pub(crate) fn info_for_comment(
    tree: &[CommentNode],
    target: u64,
) -> Option<(usize, Vec<u64>, bool, bool)> {
    for node in tree {
        if node.comment.id == target {
            return Some((
                node.comment.depth,
                node.comment.kids.clone(),
                node.comment.children_loaded,
                node.comment.children_loading,
            ));
        }
        if let Some(found) = info_for_comment(&node.children, target) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn set_children_loading(
    tree: &mut [CommentNode],
    target: u64,
    loading: bool,
) -> Option<()> {
    for node in tree {
        if node.comment.id == target {
            node.comment.children_loading = loading;
            return Some(());
        }
        if set_children_loading(&mut node.children, target, loading).is_some() {
            return Some(());
        }
    }
    None
}

pub(crate) fn attach_children(
    tree: &mut [CommentNode],
    target: u64,
    children: Vec<CommentNode>,
) -> Option<()> {
    fn inner(
        tree: &mut [CommentNode],
        target: u64,
        children: &mut Option<Vec<CommentNode>>,
    ) -> bool {
        for node in tree {
            if node.comment.id == target {
                node.children = children.take().expect("children not yet taken");
                node.comment.children_loaded = true;
                node.comment.children_loading = false;
                return true;
            }
            if inner(&mut node.children, target, children) {
                return true;
            }
        }
        false
    }

    let mut children = Some(children);
    inner(tree, target, &mut children).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(id: u64, depth: usize, kids: Vec<u64>) -> Comment {
        Comment {
            id,
            by: Some(format!("u{id}")),
            time: Some(1),
            text: format!("c{id}"),
            kids: kids.clone(),
            depth,
            collapsed: !kids.is_empty(),
            children_loaded: kids.is_empty(),
            children_loading: false,
        }
    }

    fn node(id: u64, depth: usize, kids: Vec<u64>, children: Vec<CommentNode>) -> CommentNode {
        CommentNode {
            comment: comment(id, depth, kids),
            children,
        }
    }

    fn tree() -> Vec<CommentNode> {
        vec![node(
            1,
            0,
            vec![2, 3],
            vec![
                node(2, 1, vec![4], vec![node(4, 2, vec![], vec![])]),
                node(3, 1, vec![], vec![]),
            ],
        )]
    }

    #[test]
    fn flatten_respects_collapsed_nodes() {
        let mut tree = tree();
        assert_eq!(
            flatten_visible_comments(&tree)
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            vec![1]
        );

        set_collapse(&mut tree, 1, false).expect("root present");
        assert_eq!(
            flatten_visible_comments(&tree)
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn apply_default_expansion_expands_only_configured_depths() {
        let mut tree = tree();
        apply_default_expansion(&mut tree, 2);

        assert!(!tree[0].comment.collapsed);
        assert!(tree[0].children[0].comment.collapsed);
    }

    #[test]
    fn attach_children_marks_parent_loaded_and_clear_loading() {
        let mut tree = vec![node(1, 0, vec![2], vec![])];
        set_children_loading(&mut tree, 1, true).expect("root present");

        attach_children(&mut tree, 1, vec![node(2, 1, vec![], vec![])]).expect("root present");

        assert!(tree[0].comment.children_loaded);
        assert!(!tree[0].comment.children_loading);
        assert_eq!(tree[0].children[0].comment.id, 2);
    }

    #[test]
    fn missing_comment_returns_none() {
        let mut tree = tree();

        assert!(set_collapse(&mut tree, 99, false).is_none());
        assert!(set_children_loading(&mut tree, 99, true).is_none());
        assert!(attach_children(&mut tree, 99, vec![]).is_none());
        assert!(info_for_comment(&tree, 99).is_none());
    }
}
