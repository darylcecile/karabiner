//! Copy / paste of stages between workflows and projects. The payload is versioned JSON on
//! the system clipboard and carries the agents, skills and instruction files the stages use,
//! so a paste into another project resolves cleanly. Pasted stages and edges get new ids;
//! library items are reused when identical, added when missing, and re-identified when the id
//! exists with different content. Unknown fields travel in each item's `extra`.

use std::collections::{BTreeMap, HashMap};

use protocol::{
    AgentDefinition, ForkBranch, InstructionDefinition, LayoutPosition, ProjectConfig, SkillDefinition, StageDefinition, StageOutcome,
    WorkflowDefinition, WorkflowEdge, WorkflowFork,
};
use serde::{Deserialize, Serialize};

use super::{fork, graph};
use crate::views::tasks::new_id;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    #[serde(rename = "karabinerFactory")]
    version: u32,
    stages: Vec<StageDefinition>,
    edges: Vec<WorkflowEdge>,
    /// Parallel regions wholly inside the copy (fork, branches and join).
    #[serde(default)]
    forks: Vec<WorkflowFork>,
    positions: BTreeMap<String, LayoutPosition>,
    agents: Vec<AgentDefinition>,
    skills: Vec<SkillDefinition>,
    instructions: Vec<InstructionDefinition>,
}

const VERSION: u32 = 1;

impl Clip {
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Every stage the copy holds (the selection plus any whole parallel regions).
    pub fn stage_ids(&self) -> Vec<String> {
        self.stages.iter().map(|stage| stage.id.clone()).collect()
    }

    /// Top-left of the copied stages.
    pub fn origin(&self) -> Option<LayoutPosition> {
        self.positions.values().fold(None, |acc: Option<LayoutPosition>, at| Some(acc.map_or(*at, |min| LayoutPosition { x: min.x.min(at.x), y: min.y.min(at.y) })))
    }

    pub fn from_text(text: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text).ok().filter(|clip| clip.version == VERSION && !clip.stages.is_empty())
    }
}

/// Selected stages plus, for any selected fork or join, its whole parallel region (nested
/// regions included), so a fork never pastes as an ordinary stage.
fn closure(flow: &WorkflowDefinition, selected: &[String]) -> Vec<String> {
    let forks = fork::forks(flow);
    let forward: Vec<WorkflowEdge> = graph::normalized(flow).edges.into_iter().chain(fork::branch_edges(flow))
        .filter(|edge| edge.on != StageOutcome::Return).collect();
    let mut ids: Vec<String> = selected.to_vec();
    let mut index = 0;
    while index < ids.len() {
        let id = ids[index].clone();
        index += 1;
        let Some(region) = forks.iter().find(|item| item.stage_id == id || item.join_stage_id == id) else { continue };
        // Fork, join and every stage between a branch entry and the join.
        let mut region = vec![region.stage_id.clone(), region.join_stage_id.clone()];
        let mut stack: Vec<String> = forks.iter().filter(|item| item.stage_id == region[0]).flat_map(|item| item.branches.iter().map(|branch| branch.entry_stage_id.clone())).collect();
        while let Some(stage) = stack.pop() {
            if region.contains(&stage) { continue }
            stack.extend(forward.iter().filter(|edge| edge.source_stage_id == stage).map(|edge| edge.target_stage_id.clone()));
            region.push(stage);
        }
        for stage in region { if !ids.contains(&stage) { ids.push(stage) } }
    }
    ids
}

/// The selected stages of a workflow, their edges among themselves and what they reference.
pub fn copy(config: &ProjectConfig, workflow: usize, stage_ids: &[String]) -> Option<Clip> {
    let flow = config.workflows.get(workflow)?;
    let stage_ids = &closure(flow, stage_ids);
    let stages: Vec<StageDefinition> = flow.stages.iter().filter(|stage| stage_ids.contains(&stage.id)).cloned().collect();
    if stages.is_empty() { return None }
    let inside = |id: &String| stages.iter().any(|stage| &stage.id == id);
    let edges = graph::normalized(flow).edges.into_iter().filter(|edge| inside(&edge.source_stage_id) && inside(&edge.target_stage_id)).collect();
    let forks = fork::forks(flow).into_iter().filter(|item| inside(&item.stage_id) && inside(&item.join_stage_id)).map(|mut item| {
        item.branches.retain(|branch| inside(&branch.entry_stage_id));
        item
    }).collect();
    let positions = flow.stages.iter().enumerate().filter(|(_, stage)| inside(&stage.id))
        .map(|(index, stage)| (stage.id.clone(), graph::position(flow, index))).collect();
    let agents: Vec<AgentDefinition> = config.agents.iter().filter(|agent| stages.iter().any(|stage| stage.agent_ids.contains(&agent.id))).cloned().collect();
    let used_instruction = |id: &String| stages.iter().chain([]).any(|stage| stage.instruction_ids.as_ref().is_some_and(|ids| ids.contains(id)))
        || agents.iter().any(|agent| agent.instruction_ids.as_ref().is_some_and(|ids| ids.contains(id)));
    let skills = config.skills.iter().filter(|skill| agents.iter().any(|agent| agent.skill_ids.contains(&skill.id))).cloned().collect();
    let instructions = config.instructions.iter().filter(|instruction| used_instruction(&instruction.id)).cloned().collect();
    Some(Clip { version: VERSION, stages, edges, forks, positions, agents, skills, instructions })
}

/// Adds library items, returning old id → id in this config.
fn merge<T: Clone + PartialEq>(existing: &mut Vec<T>, items: Vec<T>, id: impl Fn(&mut T) -> &mut String, prefix: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for mut item in items {
        let old = id(&mut item).clone();
        let found = existing.iter_mut().find_map(|candidate| (*id(candidate) == old).then(|| candidate.clone()));
        match found {
            Some(same) if same == item => {}
            Some(_) => { *id(&mut item) = new_id(prefix); existing.push(item.clone()) }
            None => existing.push(item.clone()),
        }
        map.insert(old, id(&mut item).clone());
    }
    map
}

fn remap(ids: &mut Vec<String>, map: &HashMap<String, String>) {
    for id in ids.iter_mut() { if let Some(new) = map.get(id) { *id = new.clone() } }
}

/// Pastes into `workflow` with the clip's top-left at `anchor`; returns the new stage ids.
pub fn paste(config: &mut ProjectConfig, workflow: usize, clip: Clip, anchor: LayoutPosition) -> Vec<String> {
    if workflow >= config.workflows.len() { return Vec::new() }
    let origin = clip.origin().unwrap_or(anchor);
    let instructions = merge(&mut config.instructions, clip.instructions, |item| &mut item.id, "instr");
    let skills = merge(&mut config.skills, clip.skills, |item| &mut item.id, "skill");
    let agents: Vec<AgentDefinition> = clip.agents.into_iter().map(|mut agent| {
        remap(&mut agent.skill_ids, &skills);
        if let Some(ids) = agent.instruction_ids.as_mut() { remap(ids, &instructions) }
        agent
    }).collect();
    let agents = merge(&mut config.agents, agents, |item| &mut item.id, "agent");
    let stage_ids: HashMap<String, String> = clip.stages.iter().map(|stage| (stage.id.clone(), new_id("stage"))).collect();
    let flow = &mut config.workflows[workflow];
    for (index, mut stage) in clip.stages.into_iter().enumerate() {
        let at = clip.positions.get(&stage.id).copied().unwrap_or(LayoutPosition { x: origin.x, y: origin.y + 120. * index as f64 });
        stage.id = stage_ids[&stage.id].clone();
        remap(&mut stage.agent_ids, &agents);
        if let Some(ids) = stage.instruction_ids.as_mut() { remap(ids, &instructions) }
        // Return targets are rebuilt from the copied edges below.
        stage.return_to_stage_ids.clear();
        graph::add_stage(flow, stage, LayoutPosition { x: anchor.x + at.x - origin.x, y: anchor.y + at.y - origin.y });
    }
    // Forks first: a fork's success link is a branch, not an ordinary edge.
    for item in clip.forks {
        let (Some(stage), Some(join)) = (stage_ids.get(&item.stage_id), stage_ids.get(&item.join_stage_id)) else { continue };
        let branches = item.branches.into_iter().filter_map(|branch| Some(ForkBranch {
            id: new_id("branch"), entry_stage_id: stage_ids.get(&branch.entry_stage_id)?.clone(), extra: branch.extra,
        })).collect();
        graph::graph_mut(flow).forks.get_or_insert_with(Vec::new).push(WorkflowFork {
            id: new_id("fork"), stage_id: stage.clone(), branches, join_stage_id: join.clone(), extra: item.extra,
        });
    }
    // Forward routes first: a return is only valid once the path it loops back over exists.
    let (forward, returns): (Vec<_>, Vec<_>) = clip.edges.into_iter().partition(|edge| edge.on != StageOutcome::Return);
    for edge in forward.into_iter().chain(returns) {
        if let (Some(source), Some(target)) = (stage_ids.get(&edge.source_stage_id), stage_ids.get(&edge.target_stage_id)) {
            // Copied edges were valid among themselves; the guard still applies.
            let _ = graph::connect(flow, source, target, edge.on);
        }
    }
    stage_ids.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{AgentMode, ExecutionTarget, HarnessId, StageOutcome, WorkflowDefinition};

    fn stage(id: &str, agents: &[&str]) -> StageDefinition {
        StageDefinition {
            id: id.into(), name: id.into(), instructions: String::new(), agent_ids: agents.iter().map(|a| a.to_string()).collect(),
            return_to_stage_ids: Vec::new(), exit_rule: None, status: None, color: None, instruction_ids: None,
            execution: None, max_visits: None, extra: Default::default(),
        }
    }

    fn agent(id: &str, model: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.into(), name: id.into(), role: String::new(), harness: HarnessId::Claude, model: model.into(), instructions: String::new(),
            skill_ids: Vec::new(), mode: AgentMode::Autonomous, executable: None, args: None, instruction_ids: None, protocol: None, strands: None, extra: Default::default(),
        }
    }

    fn config(agents: Vec<AgentDefinition>, stages: Vec<StageDefinition>) -> ProjectConfig {
        ProjectConfig {
            revision: 1, instructions: Vec::new(), skills: Vec::new(), agents, execution_target: ExecutionTarget::Host,
            target_options: None, scheduler: None, integration: None, extra: Default::default(),
            workflows: vec![WorkflowDefinition {
                id: "wf".into(), name: "W".into(), description: String::new(), instructions: String::new(), stages,
                instruction_ids: None, graph: None, layout: None, extra: Default::default(),
            }],
        }
    }

    #[test]
    fn paste_into_another_project_renames_conflicting_agents_and_keeps_edges() {
        let source = config(vec![agent("a", "opus")], vec![stage("plan", &["a"]), stage("build", &[])]);
        let clip = Clip::from_text(&copy(&source, 0, &["plan".into(), "build".into()]).unwrap().to_text()).unwrap();
        let mut target = config(vec![agent("a", "other")], Vec::new());
        let pasted = paste(&mut target, 0, clip, LayoutPosition { x: 0., y: 0. });
        assert_eq!(pasted.len(), 2);
        assert_eq!(target.agents.len(), 2);
        let plan = target.workflows[0].stages.iter().find(|stage| stage.name == "plan").unwrap();
        assert_ne!(plan.agent_ids[0], "a");
        assert_eq!(target.agents.iter().find(|agent| agent.id == plan.agent_ids[0]).unwrap().model, "opus");
        let edges = graph::normalized(&target.workflows[0]).edges;
        assert!(edges.iter().any(|edge| edge.source_stage_id == plan.id && edge.on == StageOutcome::Success));
    }

    /// Stored edge order can list a return before the forward route it loops over.
    #[test]
    fn returns_listed_before_their_forward_route_survive_paste() {
        let mut source = config(Vec::new(), vec![stage("x", &[]), stage("y", &[])]);
        graph::connect(&mut source.workflows[0], "x", "y", StageOutcome::Success).unwrap();
        graph::connect(&mut source.workflows[0], "y", "x", StageOutcome::Return).unwrap();
        source.workflows[0].graph.as_mut().unwrap().edges.reverse();
        let clip = copy(&source, 0, &["x".into(), "y".into()]).unwrap();
        let mut target = config(Vec::new(), Vec::new());
        paste(&mut target, 0, clip, LayoutPosition { x: 0., y: 0. });
        let edges = graph::normalized(&target.workflows[0]).edges;
        assert!(edges.iter().any(|edge| edge.on == StageOutcome::Return));
        assert_eq!(target.workflows[0].stages.iter().filter(|stage| !stage.return_to_stage_ids.is_empty()).count(), 1);
    }

    #[test]
    fn copying_a_join_copies_its_whole_parallel_region_with_new_ids() {
        let mut source = config(Vec::new(), vec![stage("a", &[]), stage("b", &[])]);
        let flow = &mut source.workflows[0];
        fork::add_parallel(flow, stage("split", &[]), stage("merge", &[]), LayoutPosition { x: 0., y: 0. });
        graph::connect(flow, "split", "a", StageOutcome::Success).unwrap();
        graph::connect(flow, "split", "b", StageOutcome::Success).unwrap();
        graph::connect(flow, "a", "merge", StageOutcome::Success).unwrap();
        graph::connect(flow, "b", "merge", StageOutcome::Success).unwrap();
        let clip = copy(&source, 0, &["merge".into()]).unwrap();
        assert_eq!(clip.stages.len(), 4);
        // Cut removes exactly what was copied: the whole region, no orphaned branch stages.
        let mut cut = source.workflows[0].clone();
        graph::remove_stages(&mut cut, &clip.stage_ids());
        assert!(cut.stages.is_empty() && fork::forks(&cut).is_empty());
        let mut target = config(Vec::new(), Vec::new());
        paste(&mut target, 0, clip, LayoutPosition { x: 0., y: 0. });
        let pasted = &target.workflows[0];
        let forks = fork::forks(pasted);
        assert_eq!(forks.len(), 1);
        assert_eq!(forks[0].branches.len(), 2);
        assert_ne!(forks[0].stage_id, "split");
        assert!(pasted.stages.iter().any(|stage| stage.id == forks[0].join_stage_id));
    }
}
