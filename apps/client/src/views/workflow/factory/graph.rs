//! Pure graph edits on one workflow. The service normalises a workflow without `graph` from
//! stage order (`normalizedGraph` in `packages/contracts/src/factory-validation.ts`); the
//! editor materialises the same graph on first edit and keeps `returnToStageIds` equal to the
//! return edges, which the validator requires.

use protocol::{LayoutPosition, StageDefinition, StageOutcome, WorkflowDefinition, WorkflowEdge, WorkflowGraph, WorkflowLayout};

pub const DEFAULT_MAX_TRANSITIONS: u32 = 100;
/// Default layout for stages without a stored position: left to right (input on the left,
/// outcomes on the right). A stage's column is its longest forward path from the entry, so
/// parallel branches share columns and a join sits after its branches; stages in one column
/// stack in rows. Unreachable stages follow in a last column.
const COLUMN: f64 = 230.;
const ROW: f64 = 100.;
const ORIGIN: f64 = 40.;

pub fn edge_id(source: &str, on: &StageOutcome, target: &str) -> String {
    format!("{source}:{}:{target}", on.as_str())
}

/// The graph the service would run, derived from stage order when not stored.
pub fn normalized(workflow: &WorkflowDefinition) -> WorkflowGraph {
    if let Some(graph) = &workflow.graph {
        return graph.clone();
    }
    let mut edges = Vec::new();
    for (index, stage) in workflow.stages.iter().enumerate() {
        if let Some(next) = workflow.stages.get(index + 1) {
            edges.push(edge(&stage.id, StageOutcome::Success, &next.id));
        }
        for target in &stage.return_to_stage_ids {
            edges.push(edge(&stage.id, StageOutcome::Return, target));
        }
    }
    WorkflowGraph {
        entry_stage_id: workflow.stages.first().map(|stage| stage.id.clone()).unwrap_or_default(),
        edges,
        max_transitions: Some(DEFAULT_MAX_TRANSITIONS),
        forks: None,
        extra: Default::default(),
    }
}

fn edge(source: &str, on: StageOutcome, target: &str) -> WorkflowEdge {
    WorkflowEdge { id: edge_id(source, &on, target), source_stage_id: source.into(), target_stage_id: target.into(), on, extra: Default::default() }
}

pub(super) fn graph_mut(workflow: &mut WorkflowDefinition) -> &mut WorkflowGraph {
    if workflow.graph.is_none() {
        workflow.graph = Some(normalized(workflow));
    }
    workflow.graph.as_mut().expect("materialised above")
}

/// Position of a stage, falling back to its column in stage order.
pub fn position(workflow: &WorkflowDefinition, index: usize) -> LayoutPosition {
    let Some(stage) = workflow.stages.get(index) else { return LayoutPosition { x: ORIGIN, y: ORIGIN } };
    let stored = workflow.layout.as_ref().and_then(|layout| layout.positions.get(&stage.id).copied());
    stored.unwrap_or_else(|| default_layout(workflow).get(&stage.id).copied().unwrap_or(LayoutPosition { x: ORIGIN, y: ORIGIN }))
}

/// Left-to-right positions for every stage (see the constants above).
pub fn default_layout(workflow: &WorkflowDefinition) -> std::collections::HashMap<String, LayoutPosition> {
    let graph = normalized(workflow);
    let forward: Vec<WorkflowEdge> = graph.edges.iter().cloned().chain(super::fork::branch_edges(workflow))
        .filter(|edge| edge.on != StageOutcome::Return).collect();
    // Longest-path depth; forward edges are acyclic, and the visit cap guards a bad config.
    let mut depth: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    if !graph.entry_stage_id.is_empty() { depth.insert(graph.entry_stage_id.as_str(), 0); }
    for _ in 0..workflow.stages.len() {
        let mut changed = false;
        for edge in &forward {
            let Some(&from) = depth.get(edge.source_stage_id.as_str()) else { continue };
            let next = depth.entry(edge.target_stage_id.as_str()).or_insert(0);
            if *next < from + 1 && edge.target_stage_id != graph.entry_stage_id { *next = from + 1; changed = true }
        }
        if !changed { break }
    }
    let last = depth.values().max().map_or(0, |max| max + 1);
    let mut rows: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    workflow.stages.iter().map(|stage| {
        let column = depth.get(stage.id.as_str()).copied().unwrap_or(last);
        let row = rows.entry(column).or_insert(0);
        let at = LayoutPosition { x: ORIGIN + COLUMN * column as f64, y: ORIGIN + ROW * *row as f64 };
        *row += 1;
        (stage.id.clone(), at)
    }).collect()
}

/// Drops saved positions so every stage takes the default left-to-right layout.
pub fn arrange(workflow: &mut WorkflowDefinition) {
    if let Some(layout) = workflow.layout.as_mut() { layout.positions.clear() }
}

pub fn set_position(workflow: &mut WorkflowDefinition, stage_id: &str, at: LayoutPosition) {
    workflow.layout.get_or_insert_with(WorkflowLayout::default).positions.insert(stage_id.into(), at);
}

/// Adds a stage at `at`. The first stage becomes the entry.
pub fn add_stage(workflow: &mut WorkflowDefinition, stage: StageDefinition, at: LayoutPosition) {
    let id = stage.id.clone();
    graph_mut(workflow);
    workflow.stages.push(stage);
    set_position(workflow, &id, at);
    let graph = graph_mut(workflow);
    if graph.entry_stage_id.is_empty() {
        graph.entry_stage_id = id;
    }
}

/// Success and failure each select one successor, so a new edge replaces the source's edge of
/// that kind. A stage may return to several distinct stages (the run names the one it takes).
/// Mirrors the service validator before mutating: success/failure edges stay acyclic, and a
/// return targets this stage or one that reaches it forward. The service remains the authority.
pub fn connect(workflow: &mut WorkflowDefinition, source: &str, target: &str, on: StageOutcome) -> Result<String, &'static str> {
    if !has_stage(workflow, source) || !has_stage(workflow, target) {
        return Err("That stage no longer exists.");
    }
    // A fork owns its success routing: a success link from it adds a parallel branch.
    if on == StageOutcome::Success && super::fork::fork_at(workflow, source).is_some() {
        return super::fork::add_branch(workflow, source, target);
    }
    let returns = on == StageOutcome::Return;
    let replaced = |edge: &WorkflowEdge| edge.source_stage_id == source && edge.on == on && (!returns || edge.target_stage_id == target);
    let forward: Vec<WorkflowEdge> = normalized(workflow).edges.into_iter().chain(super::fork::branch_edges(workflow))
        .filter(|edge| edge.on != StageOutcome::Return && !replaced(edge)).collect();
    if returns {
        if source != target && !reaches(&forward, target, source) {
            return Err("A return route must go back to this stage or one that leads to it.");
        }
    } else if source == target || reaches(&forward, target, source) {
        return Err("Success and failure routes can't loop back; use a return route for rework.");
    }
    let graph = graph_mut(workflow);
    graph.edges.retain(|edge| !replaced(edge));
    let added = edge(source, on, target);
    let id = added.id.clone();
    graph.edges.push(added);
    sync_returns(workflow);
    Ok(id)
}

/// Whether `to` is reachable from `from` along `edges`.
fn reaches(edges: &[WorkflowEdge], from: &str, to: &str) -> bool {
    let (mut stack, mut seen) = (vec![from], std::collections::HashSet::new());
    while let Some(id) = stack.pop() {
        if id == to { return true }
        if seen.insert(id) {
            stack.extend(edges.iter().filter(|edge| edge.source_stage_id == id).map(|edge| edge.target_stage_id.as_str()));
        }
    }
    false
}

/// Changes an edge's outcome, replacing any edge of that kind from the same source.
/// Changes an edge's outcome; on refusal the workflow is left unchanged.
pub fn set_outcome(workflow: &mut WorkflowDefinition, edge_id: &str, on: StageOutcome) -> Result<String, &'static str> {
    let edge = normalized(workflow).edges.into_iter().find(|edge| edge.id == edge_id).ok_or("That route no longer exists.")?;
    let mut candidate = workflow.clone();
    remove_edges(&mut candidate, &[edge_id.to_string()]);
    let id = connect(&mut candidate, &edge.source_stage_id, &edge.target_stage_id, on)?;
    *workflow = candidate;
    Ok(id)
}

pub fn remove_edges(workflow: &mut WorkflowDefinition, ids: &[String]) {
    super::fork::remove_branches(workflow, ids);
    graph_mut(workflow).edges.retain(|edge| !ids.contains(&edge.id));
    sync_returns(workflow);
}

/// Removes stages with their edges and positions; the entry moves to the first remaining stage.
pub fn remove_stages(workflow: &mut WorkflowDefinition, ids: &[String]) {
    graph_mut(workflow);
    workflow.stages.retain(|stage| !ids.contains(&stage.id));
    if let Some(layout) = workflow.layout.as_mut() {
        layout.positions.retain(|id, _| !ids.contains(id));
    }
    let first = workflow.stages.first().map(|stage| stage.id.clone()).unwrap_or_default();
    let graph = graph_mut(workflow);
    graph.edges.retain(|edge| !ids.contains(&edge.source_stage_id) && !ids.contains(&edge.target_stage_id));
    if ids.contains(&graph.entry_stage_id) {
        graph.entry_stage_id = first;
    }
    super::fork::prune(workflow);
    sync_returns(workflow);
}

pub fn set_entry(workflow: &mut WorkflowDefinition, stage_id: &str) {
    if has_stage(workflow, stage_id) {
        graph_mut(workflow).entry_stage_id = stage_id.into();
    }
}

pub fn set_max_transitions(workflow: &mut WorkflowDefinition, value: Option<u32>) {
    graph_mut(workflow).max_transitions = value;
}

fn has_stage(workflow: &WorkflowDefinition, id: &str) -> bool {
    workflow.stages.iter().any(|stage| stage.id == id)
}

fn sync_returns(workflow: &mut WorkflowDefinition) {
    let edges = normalized(workflow).edges;
    for stage in workflow.stages.iter_mut() {
        stage.return_to_stage_ids = edges.iter()
            .filter(|edge| edge.source_stage_id == stage.id && edge.on == StageOutcome::Return)
            .map(|edge| edge.target_stage_id.clone())
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(id: &str) -> StageDefinition {
        StageDefinition {
            id: id.into(), name: id.into(), instructions: String::new(), agent_ids: Vec::new(), return_to_stage_ids: Vec::new(),
            exit_rule: None, status: None, color: None, instruction_ids: None, execution: None, max_visits: None, extra: Default::default(),
        }
    }

    fn workflow() -> WorkflowDefinition {
        let mut build = stage("build");
        build.return_to_stage_ids = vec!["plan".into()];
        WorkflowDefinition {
            id: "wf".into(), name: "W".into(), description: String::new(), instructions: String::new(),
            stages: vec![stage("plan"), build, stage("ship")], instruction_ids: None, graph: None, layout: None, extra: Default::default(),
        }
    }

    #[test]
    fn materialises_the_service_graph_then_replaces_same_outcome() {
        let mut flow = workflow();
        connect(&mut flow, "plan", "ship", StageOutcome::Success).unwrap();
        assert!(connect(&mut flow, "ship", "plan", StageOutcome::Failure).is_err());
        let graph = flow.graph.clone().unwrap();
        assert_eq!(graph.entry_stage_id, "plan");
        let from_plan: Vec<_> = graph.edges.iter().filter(|edge| edge.source_stage_id == "plan").map(|edge| edge.target_stage_id.as_str()).collect();
        assert_eq!(from_plan, ["ship"]);
        assert!(graph.edges.iter().any(|edge| edge.id == "build:return:plan"));
    }

    #[test]
    fn default_layout_runs_left_to_right_with_branches_sharing_a_column() {
        let mut flow = workflow();
        super::super::fork::add_parallel(&mut flow, stage("split"), stage("merge"), LayoutPosition { x: 0., y: 0. });
        add_stage(&mut flow, stage("x"), LayoutPosition { x: 0., y: 0. });
        add_stage(&mut flow, stage("y"), LayoutPosition { x: 0., y: 0. });
        for (from, to) in [("ship", "split"), ("split", "x"), ("split", "y"), ("x", "merge"), ("y", "merge")] {
            connect(&mut flow, from, to, StageOutcome::Success).unwrap();
        }
        arrange(&mut flow);
        let at = |id: &str| position(&flow, flow.stages.iter().position(|stage| stage.id == id).unwrap());
        let order = ["plan", "build", "ship", "split", "x", "merge"];
        assert!(order.windows(2).all(|pair| at(pair[0]).x < at(pair[1]).x));
        assert_eq!(at("x").x, at("y").x);
        assert_ne!(at("x").y, at("y").y);
    }

    #[test]
    fn return_ids_follow_return_edges_and_stage_removal() {
        let mut flow = workflow();
        connect(&mut flow, "ship", "build", StageOutcome::Return).unwrap();
        connect(&mut flow, "ship", "plan", StageOutcome::Return).unwrap();
        assert_eq!(flow.stages[2].return_to_stage_ids, ["build", "plan"]);
        remove_stages(&mut flow, &["plan".into()]);
        assert_eq!(flow.graph.as_ref().unwrap().entry_stage_id, "build");
        assert!(flow.stages[0].return_to_stage_ids.is_empty());
        assert_eq!(flow.stages[1].return_to_stage_ids, ["build"]);
    }
}
