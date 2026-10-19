//! Flattens a snapshot into the text a person would read on screen.

use crate::snapshot::{AppSnapshot, Node, Rect};

/// Roles whose `AXValue` is visible text.
const TEXT_ROLES: &[&str] = &[
    "AXStaticText",
    "AXTextArea",
    "AXTextField",
    "AXComboBox",
    "AXCell",
];

/// Web pages mark their primary content with `<main>` or `role="main"`;
/// browsers expose it with this subrole.
const MAIN_LANDMARK: &str = "AXLandmarkMain";

const SCROLL_AREA: &str = "AXScrollArea";

/// When a window marks a main content area, only that area is used, which
/// drops sidebars, banners and toolbars. Otherwise the whole window is used.
pub fn to_text(snapshot: &AppSnapshot) -> String {
    lines(snapshot, false)
}

/// The part of `to_text` the window shows: text inside the window and inside
/// every scroll area around it. Text scrolled out of sight is left out, and
/// so is a window whose frame is not known.
pub fn in_view_text(snapshot: &AppSnapshot) -> String {
    lines(snapshot, true)
}

fn lines(snapshot: &AppSnapshot, in_view: bool) -> String {
    let mut lines: Vec<String> = Vec::new();
    for window in &snapshot.windows {
        let view = match (in_view, window.root.frame) {
            (false, _) => None,
            (true, Some(frame)) if has_area(&frame) => Some(frame),
            (true, _) => continue,
        };
        if let Some(title) = &window.title {
            lines.push(format!("# {title}"));
        }
        let mut mains = Vec::new();
        find_main_landmarks(&window.root, view, &mut mains);
        if mains.is_empty() {
            visit(&window.root, view, &mut lines);
        } else {
            for (main, view) in mains {
                visit(main, view, &mut lines);
            }
        }
    }
    lines.dedup();
    lines.join("\n")
}

/// Main landmarks, each with the part of the window in view where it sits
/// (`None` when reading everything).
fn find_main_landmarks<'a>(
    node: &'a Node,
    view: Option<Rect>,
    found: &mut Vec<(&'a Node, Option<Rect>)>,
) {
    let Some(view) = narrow(node, view) else {
        return;
    };
    if node.subrole.as_deref() == Some(MAIN_LANDMARK) {
        found.push((node, view));
        return;
    }
    for child in &node.children {
        find_main_landmarks(child, view, found);
    }
}

/// The part of the window in view inside `node`: `None` if the node is out
/// of sight, otherwise the view, narrowed to the node if it scrolls. Without
/// a view (reading everything) nothing is out of sight. A node without a
/// frame, or with an empty one (some groups), is judged by its children.
#[allow(clippy::option_option)]
fn narrow(node: &Node, view: Option<Rect>) -> Option<Option<Rect>> {
    let Some(view) = view else {
        return Some(None);
    };
    let Some(frame) = node.frame.filter(has_area) else {
        return Some(Some(view));
    };
    let inside = overlap(&frame, &view)?;
    Some(Some(if node.role == SCROLL_AREA {
        inside
    } else {
        view
    }))
}

fn has_area(rect: &Rect) -> bool {
    rect.width > 0.0 && rect.height > 0.0
}

/// Where two rectangles overlap, if they do by more than an edge.
fn overlap(a: &Rect, b: &Rect) -> Option<Rect> {
    let (left, top) = (a.x.max(b.x), a.y.max(b.y));
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    let rect = Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    has_area(&rect).then_some(rect)
}

fn visit(node: &Node, view: Option<Rect>, lines: &mut Vec<String>) {
    let Some(view) = narrow(node, view) else {
        return;
    };
    // Mac Catalyst apps (WhatsApp, for one) put text in descriptions. A
    // text view that scrolls shows only part of its value.
    let shown = view.and(node.shown.as_ref());
    let text = if TEXT_ROLES.contains(&node.role.as_str()) {
        shown
            .or(node.value.as_ref())
            .or(node.title.as_ref())
            .or(node.description.as_ref())
    } else if node.role == "AXHeading" {
        node.title.as_ref().or(node.description.as_ref())
    } else {
        None
    };
    if let Some(text) = text {
        lines.extend(text.lines().map(str::trim_end).map(str::to_owned));
    }
    for child in &node.children {
        visit(child, view, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Stats, WindowSnapshot};

    fn node(role: &str, subrole: Option<&str>, value: Option<&str>, children: Vec<Node>) -> Node {
        Node {
            role: role.to_owned(),
            subrole: subrole.map(str::to_owned),
            title: None,
            value: value.map(str::to_owned),
            description: None,
            identifier: None,
            classes: Vec::new(),
            url: None,
            frame: None,
            shown: None,
            children,
        }
    }

    fn text(value: &str) -> Node {
        node("AXStaticText", None, Some(value), vec![])
    }

    fn at(mut node: Node, x: f64, y: f64, width: f64, height: f64) -> Node {
        node.frame = Some(Rect {
            x,
            y,
            width,
            height,
        });
        node
    }

    fn snapshot(root: Node) -> AppSnapshot {
        AppSnapshot {
            pid: 1,
            app_name: None,
            windows: vec![WindowSnapshot {
                title: Some("Page".to_owned()),
                root,
            }],
            stats: Stats::default(),
        }
    }

    #[test]
    fn keeps_only_the_main_landmark_when_present() {
        let root = node(
            "AXWindow",
            None,
            None,
            vec![
                node("AXGroup", None, None, vec![text("Sidebar"), text("99+")]),
                node(
                    "AXGroup",
                    Some(MAIN_LANDMARK),
                    None,
                    vec![text("Article title"), text("Article title"), text("Body")],
                ),
            ],
        );
        assert_eq!(to_text(&snapshot(root)), "# Page\nArticle title\nBody");
    }

    #[test]
    fn in_view_leaves_out_text_scrolled_out_of_sight() {
        // A 1000 x 800 window: a toolbar above a scroll area that shows
        // y 100 to 700 of a longer document.
        let root = at(
            node(
                "AXWindow",
                None,
                None,
                vec![
                    at(text("Toolbar"), 0.0, 40.0, 200.0, 20.0),
                    at(
                        node(
                            "AXScrollArea",
                            None,
                            None,
                            vec![
                                at(text("Scrolled past"), 0.0, -300.0, 900.0, 20.0),
                                at(text("Showing"), 0.0, 400.0, 900.0, 20.0),
                                // Cut by the bottom edge: still partly in view.
                                at(text("Half shown"), 0.0, 690.0, 900.0, 20.0),
                                // Inside the window, below the scroll area.
                                at(text("Below the fold"), 0.0, 750.0, 900.0, 20.0),
                                // No frame: judged by its parent.
                                node(
                                    "AXGroup",
                                    None,
                                    None,
                                    vec![
                                        text("Unplaced"),
                                        at(text("Far below"), 0.0, 5000.0, 900.0, 20.0),
                                    ],
                                ),
                            ],
                        ),
                        0.0,
                        100.0,
                        1000.0,
                        600.0,
                    ),
                ],
            ),
            0.0,
            0.0,
            1000.0,
            800.0,
        );
        let snap = snapshot(root);
        assert_eq!(
            in_view_text(&snap),
            "# Page\nToolbar\nShowing\nHalf shown\nUnplaced"
        );
        // Reading everything still reads it all.
        assert_eq!(
            to_text(&snap),
            "# Page\nToolbar\nScrolled past\nShowing\nHalf shown\nBelow the fold\nUnplaced\nFar below"
        );
    }

    #[test]
    fn a_long_text_view_shows_only_its_visible_part() {
        let mut document = at(
            node("AXTextArea", None, Some("Line 1\nLine 2\nLine 3"), vec![]),
            0.0,
            0.0,
            500.0,
            5000.0,
        );
        document.shown = Some("Line 2".to_owned());
        let root = at(
            node("AXWindow", None, None, vec![document]),
            0.0,
            0.0,
            500.0,
            400.0,
        );
        let snap = snapshot(root);
        assert_eq!(in_view_text(&snap), "# Page\nLine 2");
        assert_eq!(to_text(&snap), "# Page\nLine 1\nLine 2\nLine 3");
    }

    #[test]
    fn in_view_needs_the_window_frame() {
        let snap = snapshot(node("AXWindow", None, None, vec![text("Somewhere")]));
        assert_eq!(in_view_text(&snap), "");
    }

    #[test]
    fn uses_the_whole_window_without_a_main_landmark() {
        let root = node(
            "AXWindow",
            None,
            None,
            vec![text("Toolbar"), text("Document")],
        );
        assert_eq!(to_text(&snapshot(root)), "# Page\nToolbar\nDocument");
    }
}
