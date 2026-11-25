//! Minimal Mermaid flowchart support for tour diagrams: parse nodes and labelled edges,
//! lay them out top-down in layers, and paint orthogonal connectors.
//! Anything that can't be parsed is returned as `None` so callers fall back to the source.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::{components::Typography as _, theme::metrics};

// Node geometry from `851174F7`: 40 high, width fitted to the label (13pt medium ≈ 7pt per
// character plus 16 padding each side), R8; 56 between ranks for the edge label.
const NODE_H: f32 = 40.;
const NODE_MIN_W: f32 = 140.;
const NODE_MAX_W: f32 = 260.;
const COL_GAP: f32 = 24.;
const ROW_GAP: f32 = 56.;
const ELBOW: f32 = 6.;

fn node_width(label: &str) -> f32 {
    (label.chars().count() as f32 * 7. + 32.).clamp(NODE_MIN_W, NODE_MAX_W)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

const ARROWS: [&str; 6] = ["-.->", "==>", "-->", "---", "--o", "--x"];
const STYLING: [&str; 5] = ["classDef", "class ", "style ", "linkStyle", "click "];

impl Graph {
    fn node(&mut self, token: &str) -> Option<usize> {
        let token = token.trim().trim_end_matches(';').trim();
        if token.is_empty() {
            return None;
        }
        let split = token.find(['[', '(', '{', '>']).unwrap_or(token.len());
        let id = token[..split].trim().to_owned();
        if id.is_empty() || id.contains(char::is_whitespace) {
            return None;
        }
        let label = clean_label(&token[split..]);
        if let Some(ix) = self.nodes.iter().position(|node| node.id == id) {
            if !label.is_empty() {
                self.nodes[ix].label = label;
            }
            return Some(ix);
        }
        let label = if label.is_empty() { id.clone() } else { label };
        self.nodes.push(Node { id, label });
        Some(self.nodes.len() - 1)
    }

    fn statement(&mut self, line: &str) -> Option<()> {
        let Some((at, arrow)) = first_arrow(line) else {
            self.node(line)?;
            return Some(());
        };
        let (mut left, mut label) = (line[..at].trim(), None);
        if let Some(split) = left.rfind(" -- ").or_else(|| left.rfind(" == ")) {
            label = Some(left[split + 4..].trim().to_owned());
            left = &left[..split];
        }
        let mut right = line[at + arrow.len()..].trim();
        if let Some(rest) = right.strip_prefix('|') {
            let end = rest.find('|')?;
            label = Some(rest[..end].trim().to_owned());
            right = rest[end + 1..].trim();
        }
        let from = self.node(left)?;
        let next_arrow = first_arrow(right).map(|(ix, _)| ix);
        let target = next_arrow.map_or(right, |ix| right[..ix].trim_end_matches(" --").trim());
        let to = self.node(target)?;
        let label = label.filter(|text| !text.is_empty()).map(|text| clean_label(&text));
        self.edges.push(Edge { from, to, label });
        if next_arrow.is_some() {
            self.statement(right)?;
        }
        Some(())
    }
}

fn first_arrow(line: &str) -> Option<(usize, &'static str)> {
    ARROWS
        .iter()
        .filter_map(|arrow| line.find(arrow).map(|ix| (ix, *arrow)))
        .min_by_key(|(ix, arrow)| (*ix, usize::MAX - arrow.len()))
}

fn clean_label(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| matches!(c, '[' | ']' | '(' | ')' | '{' | '}' | '>' | '/' | '\\'))
        .trim()
        .trim_matches('"')
        .trim()
        .to_owned()
}

/// `None` when the source isn't a flowchart this parser fully understands; callers then
/// show the source instead of a partial graph. Styling statements are ignored.
pub fn parse_mermaid(source: &str) -> Option<Graph> {
    let mut statements = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("%%"))
        .flat_map(|line| line.split(';'))
        .map(str::trim)
        .filter(|statement| !statement.is_empty());
    let header = statements.next()?;
    if !(header.starts_with("graph") || header.starts_with("flowchart")) || header.split_whitespace().count() > 2 {
        return None;
    }
    let mut graph = Graph::default();
    for statement in statements {
        if STYLING.iter().any(|prefix| statement.starts_with(prefix)) {
            continue;
        }
        if statement.contains('&') || statement.starts_with("subgraph") || statement == "end" {
            return None;
        }
        graph.statement(statement)?;
    }
    (!graph.nodes.is_empty()).then_some(graph)
}

/// Longest-path layering (bounded so cycles can't loop forever).
fn layers(graph: &Graph) -> Vec<usize> {
    let mut layer = vec![0usize; graph.nodes.len()];
    for _ in 0..graph.nodes.len() {
        let mut changed = false;
        for edge in &graph.edges {
            if edge.from != edge.to && layer[edge.to] < layer[edge.from] + 1 && layer[edge.from] + 1 < graph.nodes.len() {
                layer[edge.to] = layer[edge.from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    layer
}

pub struct Layout {
    pub positions: Vec<Point<Pixels>>,
    pub widths: Vec<f32>,
    pub size: Size<Pixels>,
}

pub fn layout(graph: &Graph) -> Layout {
    let layer = layers(graph);
    let rows = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); rows];
    for (ix, row) in layer.iter().enumerate() {
        members[*row].push(ix);
    }
    let widths: Vec<f32> = graph.nodes.iter().map(|node| node_width(&node.label)).collect();
    let row_width = |nodes: &Vec<usize>| nodes.iter().map(|node| widths[*node]).sum::<f32>() + (nodes.len() as f32 - 1.).max(0.) * COL_GAP;
    let width = members.iter().map(row_width).fold(0., f32::max);
    let mut positions = vec![point(px(0.), px(0.)); graph.nodes.len()];
    for (row, nodes) in members.iter().enumerate() {
        let mut x = (width - row_width(nodes)) / 2.;
        for node in nodes {
            positions[*node] = point(px(x), px(row as f32 * (NODE_H + ROW_GAP)));
            x += widths[*node] + COL_GAP;
        }
    }
    let height = rows as f32 * NODE_H + (rows as f32 - 1.) * ROW_GAP;
    Layout { positions, widths, size: size(px(width), px(height)) }
}

pub type NodeClick = Rc<dyn Fn(&Node, &mut Window, &mut App)>;

/// Renders the graph; `on_click` is offered when a node maps to something navigable.
pub fn render(id: impl Into<SharedString>, graph: &Graph, on_click: Option<NodeClick>, cx: &App) -> impl IntoElement {
    let id: SharedString = id.into();
    let t = crate::theme::tokens(cx);
    let Layout { positions, widths, size: area } = layout(graph);
    let line = t.text_tertiary;
    let arrow = crate::theme::tokens(cx).running;
    let segments: Vec<(Point<Pixels>, Point<Pixels>)> = graph
        .edges
        .iter()
        .map(|edge| {
            let (from, to) = (positions[edge.from], positions[edge.to]);
            (point(from.x + px(widths[edge.from] / 2.), from.y + px(NODE_H)), point(to.x + px(widths[edge.to] / 2.), to.y))
        })
        .collect();
    let painted = segments.clone();
    let edges = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            for (start, end) in &painted {
                paint_connector(bounds.origin, *start, *end, line, arrow, window);
            }
        },
    )
    .absolute()
    .size_full();
    let labels = graph.edges.iter().zip(&segments).filter_map(|(edge, (start, end))| {
        let text = edge.label.clone()?;
        let mid_y = (start.y + end.y) / 2.;
        Some(
            div()
                .absolute()
                .left(end.x - px(90.))
                .top(mid_y - px(8.))
                .w(px(180.))
                .flex()
                .justify_center()
                .child(div().px_1().bg(t.tray).t_caption().font_weight(FontWeight::MEDIUM).text_color(t.text_secondary).child(text)),
        )
    });
    let nodes = graph.nodes.iter().zip(&positions).enumerate().map(|(ix, (node, position))| {
        let clickable = on_click.clone();
        let node_data = node.clone();
        div()
            .id((id.clone(), ix))
            .absolute()
            .left(position.x)
            .top(position.y)
            .w(px(widths[ix]))
            .h(px(NODE_H))
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .rounded(px(metrics::R_INNER))
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .shadow_xs()
            .t_label()
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.text)
            .text_center()
            .child(div().truncate().child(node.label.clone()))
            .when_some(clickable, |el, click| {
                el.cursor_pointer()
                    .hover(move |style| style.border_color(t.running))
                    .on_click(move |_, window, cx| click(&node_data, window, cx))
            })
    });
    div()
        .relative()
        .w(area.width)
        .h(area.height)
        .mx_auto()
        .child(edges)
        .children(labels)
        .children(nodes)
}

/// Orthogonal connector with rounded elbows and a 7×5 arrowhead in the running colour.
fn paint_connector(origin: Point<Pixels>, start: Point<Pixels>, end: Point<Pixels>, line: Hsla, arrow: Hsla, window: &mut Window) {
    let (start, end) = (origin + start, origin + end);
    let tip = end.y - px(5.);
    let mut path = PathBuilder::stroke(px(1.));
    path.move_to(start);
    let dx = end.x - start.x;
    if end.y > start.y && dx.abs() > px(0.5) {
        let mid = (start.y + end.y) / 2.;
        let r = px(ELBOW).min(dx.abs() / 2.).min((mid - start.y).max(px(0.)));
        let dir = if dx > px(0.) { 1. } else { -1. };
        path.line_to(point(start.x, mid - r));
        path.curve_to(point(start.x + r * dir, mid), point(start.x, mid));
        path.line_to(point(end.x - r * dir, mid));
        path.curve_to(point(end.x, mid + r), point(end.x, mid));
        path.line_to(point(end.x, tip));
    } else {
        path.line_to(point(end.x, tip));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, line);
    }
    let mut head = PathBuilder::fill();
    head.move_to(point(end.x - px(3.5), tip));
    head.line_to(point(end.x + px(3.5), tip));
    head.line_to(end);
    head.close();
    if let Ok(head) = head.build() {
        window.paint_path(head, arrow);
    }
}

#[cfg(test)]
mod tests {
    use super::{NODE_H, ROW_GAP, layout, parse_mermaid};
    use gpui_kit::px;

    #[test]
    fn parses_labelled_chains() {
        let graph = parse_mermaid("flowchart TD\n  A[04 · View test] -->|verifies| B(03 · Gating)\n  B -- relies on --> C\n").unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.nodes[0].label, "04 · View test");
        assert_eq!(graph.edges[0].label.as_deref(), Some("verifies"));
        assert_eq!(graph.edges[1].label.as_deref(), Some("relies on"));
        assert_eq!(layout(&graph).positions[2].y, px(2. * (NODE_H + ROW_GAP)));
    }

    #[test]
    fn header_tail_is_parsed_and_unsupported_syntax_falls_back() {
        let graph = parse_mermaid("flowchart TD; A-->B").unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert!(parse_mermaid("flowchart TD\n A & B --> C").is_none());
        assert!(parse_mermaid("flowchart TD\n subgraph x\n A-->B\n end").is_none());
        assert!(parse_mermaid("sequenceDiagram\n A->>B: hi").is_none());
    }
}
