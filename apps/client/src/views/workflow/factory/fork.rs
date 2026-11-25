//! Parallel branches (`WorkflowGraph.forks`): a fork stage fans out to each branch entry and
//! every branch meets again at the fork's join stage. The fork owns its success routing, so a
//! success link from a fork stage adds a branch instead of an ordinary edge. Branches show on
//! the canvas as success edges with ids `fork:<fork id>:<branch id>`.

use protocol::{ForkBranch, LayoutPosition, StageDefinition, WorkflowDefinition, WorkflowEdge, WorkflowFork};

use super::graph;
use crate::views::tasks::new_id;

const PREFIX: &str = "fork:";
/// Vertical gap between a new fork and its join.
const JOIN_GAP: f64 = 240.;

pub fn forks(workflow: &WorkflowDefinition) -> Vec<WorkflowFork> {
    graph::normalized(workflow).forks.unwrap_or_default()
}

pub fn fork_at(workflow: &WorkflowDefinition, stage_id: &str) -> Option<WorkflowFork> {
    forks(workflow).into_iter().find(|fork| fork.stage_id == stage_id)
}

pub fn joined_by(workflow: &WorkflowDefinition, stage_id: &str) -> Option<WorkflowFork> {
    forks(workflow).into_iter().find(|fork| fork.join_stage_id == stage_id)
}

pub fn branch_edge_id(fork: &str, branch: &str) -> String {
    format!("{PREFIX}{fork}:{branch}")
}

/// `(fork id, branch id)` for a branch edge id.
pub fn parse_branch_edge(id: &str) -> Option<(String, String)> {
    let (fork, branch) = id.strip_prefix(PREFIX)?.split_once(':')?;
    Some((fork.into(), branch.into()))
}

/// Branch links as success edges, for drawing and reachability.
pub fn branch_edges(workflow: &WorkflowDefinition) -> Vec<WorkflowEdge> {
    forks(workflow).iter().flat_map(|fork| fork.branches.iter().map(move |branch| WorkflowEdge {
        id: branch_edge_id(&fork.id, &branch.id),
        source_stage_id: fork.stage_id.clone(),
        target_stage_id: branch.entry_stage_id.clone(),
        on: protocol::StageOutcome::Success,
        extra: Default::default(),
    })).collect()
}

fn forks_mut(workflow: &mut WorkflowDefinition) -> &mut Vec<WorkflowFork> {
    graph::graph_mut(workflow).forks.get_or_insert_with(Vec::new)
}

/// Adds a fork stage and its join below it; returns the fork stage id.
pub fn add_parallel(workflow: &mut WorkflowDefinition, fork: StageDefinition, join: StageDefinition, at: LayoutPosition) -> String {
    let (fork_id, join_id) = (fork.id.clone(), join.id.clone());
    graph::add_stage(workflow, fork, at);
    graph::add_stage(workflow, join, LayoutPosition { x: at.x, y: at.y + JOIN_GAP });
    forks_mut(workflow).push(WorkflowFork {
        id: new_id("fork"), stage_id: fork_id.clone(), branches: Vec::new(), join_stage_id: join_id, extra: Default::default(),
    });
    fork_id
}

/// Adds `entry` as a branch of the fork at `fork_stage`; the fork's plain success edge goes.
pub fn add_branch(workflow: &mut WorkflowDefinition, fork_stage: &str, entry: &str) -> Result<String, &'static str> {
    let fork = fork_at(workflow, fork_stage).ok_or("That stage doesn't start parallel branches.")?;
    if entry == fork.stage_id || entry == fork.join_stage_id {
        return Err("A branch starts at a stage between the fork and its join.");
    }
    if fork.branches.iter().any(|branch| branch.entry_stage_id == entry) {
        return Err("That stage already starts a branch.");
    }
    let branch = ForkBranch { id: new_id("branch"), entry_stage_id: entry.into(), extra: Default::default() };
    let id = branch_edge_id(&fork.id, &branch.id);
    graph::graph_mut(workflow).edges.retain(|edge| !(edge.source_stage_id == fork_stage && edge.on == protocol::StageOutcome::Success));
    if let Some(fork) = forks_mut(workflow).iter_mut().find(|item| item.id == fork.id) { fork.branches.push(branch) }
    Ok(id)
}

/// Removes branch edges (ids from `branch_edge_id`); other ids are ignored.
pub fn remove_branches(workflow: &mut WorkflowDefinition, edge_ids: &[String]) {
    let targets: Vec<(String, String)> = edge_ids.iter().filter_map(|id| parse_branch_edge(id)).collect();
    if targets.is_empty() { return }
    for fork in forks_mut(workflow).iter_mut() {
        fork.branches.retain(|branch| !targets.contains(&(fork.id.clone(), branch.id.clone())));
    }
}

pub fn set_join(workflow: &mut WorkflowDefinition, fork_stage: &str, join: &str) {
    if let Some(fork) = forks_mut(workflow).iter_mut().find(|fork| fork.stage_id == fork_stage) { fork.join_stage_id = join.into() }
}

/// A fork on `stage_id` with the join chosen later; no-op when one exists.
pub fn make_fork(workflow: &mut WorkflowDefinition, stage_id: &str, join: &str) {
    if fork_at(workflow, stage_id).is_some() || stage_id == join { return }
    graph::graph_mut(workflow).edges.retain(|edge| !(edge.source_stage_id == stage_id && edge.on == protocol::StageOutcome::Success));
    forks_mut(workflow).push(WorkflowFork {
        id: new_id("fork"), stage_id: stage_id.into(), branches: Vec::new(), join_stage_id: join.into(), extra: Default::default(),
    });
}

pub fn remove_fork(workflow: &mut WorkflowDefinition, stage_id: &str) {
    forks_mut(workflow).retain(|fork| fork.stage_id != stage_id);
}

/// Drops forks whose fork or join stage is gone, and branches whose entry is gone.
pub fn prune(workflow: &mut WorkflowDefinition) {
    if graph::normalized(workflow).forks.is_none() { return }
    let ids: Vec<String> = workflow.stages.iter().map(|stage| stage.id.clone()).collect();
    let forks = forks_mut(workflow);
    forks.retain(|fork| ids.contains(&fork.stage_id) && ids.contains(&fork.join_stage_id));
    for fork in forks.iter_mut() { fork.branches.retain(|branch| ids.contains(&branch.entry_stage_id)) }
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

    #[test]
    fn success_from_a_fork_adds_branches_and_removal_prunes_them() {
        let mut flow = WorkflowDefinition {
            id: "wf".into(), name: "W".into(), description: String::new(), instructions: String::new(),
            stages: vec![stage("a"), stage("b")], instruction_ids: None, graph: None, layout: None, extra: Default::default(),
        };
        add_parallel(&mut flow, stage("split"), stage("merge"), LayoutPosition { x: 0., y: 0. });
        graph::connect(&mut flow, "split", "a", protocol::StageOutcome::Success).unwrap();
        graph::connect(&mut flow, "split", "b", protocol::StageOutcome::Success).unwrap();
        assert_eq!(fork_at(&flow, "split").unwrap().branches.len(), 2);
        assert!(graph::normalized(&flow).edges.iter().all(|edge| edge.source_stage_id != "split"));
        graph::remove_stages(&mut flow, &["a".into()]);
        assert_eq!(fork_at(&flow, "split").unwrap().branches.len(), 1);
        graph::remove_stages(&mut flow, &["merge".into()]);
        assert!(fork_at(&flow, "split").is_none());
    }
}
