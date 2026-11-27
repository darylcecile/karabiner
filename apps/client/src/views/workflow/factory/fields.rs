//! Text fields of the factory inspector: how each reads from and writes to the config.

use protocol::{ProjectConfig, SchedulerConfig, StageDefinition, StageExecution, WorkflowDefinition};

use super::{graph, hooks};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    WorkflowName,
    WorkflowInstructions,
    MaxTransitions,
    StageName,
    StageInstructions,
    ExitRule,
    MaxVisits,
    TimeoutSeconds,
    Retries,
    RetryDelaySeconds,
    Cwd,
    Before,
    Commands,
    Gates,
    After,
    MaxConcurrentRuns,
    PollSeconds,
}

impl Key {
    pub const SINGLE: [Key; 11] = [Key::Cwd, Key::WorkflowName, Key::MaxTransitions, Key::StageName, Key::ExitRule, Key::MaxVisits, Key::TimeoutSeconds, Key::Retries, Key::RetryDelaySeconds, Key::MaxConcurrentRuns, Key::PollSeconds];
    pub const MULTI: [Key; 6] = [Key::WorkflowInstructions, Key::StageInstructions, Key::Before, Key::Commands, Key::Gates, Key::After];

    pub fn placeholder(self) -> &'static str {
        match self {
            Key::WorkflowName => "Workflow name",
            Key::WorkflowInstructions => "Instructions every stage receives",
            Key::MaxTransitions => "100 (default)",
            Key::StageName => "Stage name",
            Key::StageInstructions => "What agents do in this stage",
            Key::ExitRule => "e.g. every check is green",
            Key::MaxVisits => "3 (default)",
            Key::TimeoutSeconds => "Default",
            Key::Retries => "0 (default)",
            Key::Cwd => "Project checkout",
            Key::RetryDelaySeconds => "0 (default)",
            Key::Before => "One command per line, e.g. bun install",
            Key::Commands => "One command per line, e.g. bun test",
            Key::Gates => "approval: Ready to ship?\ncommand: bun run check",
            Key::After => "One command per line",
            Key::MaxConcurrentRuns => "1 (default)",
            Key::PollSeconds => "1 (default)",
        }
    }
}

fn number<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

/// Empty clears; anything unparsable keeps the previous value.
fn parse<T: std::str::FromStr + Copy>(text: &str, previous: Option<T>) -> Option<T> {
    let text = text.trim();
    if text.is_empty() { None } else { text.parse().ok().or(previous) }
}

pub fn value(key: Key, config: &ProjectConfig, workflow: Option<&WorkflowDefinition>, stage: Option<&StageDefinition>) -> String {
    let execution = stage.and_then(|stage| stage.execution.as_ref());
    let scheduler = config.scheduler.as_ref();
    match key {
        Key::WorkflowName => workflow.map_or_else(String::new, |workflow| workflow.name.clone()),
        Key::WorkflowInstructions => workflow.map_or_else(String::new, |workflow| workflow.instructions.clone()),
        // The saved value only, so an unset limit shows the default placeholder.
        Key::MaxTransitions => number(workflow.and_then(|workflow| workflow.graph.as_ref()).and_then(|graph| graph.max_transitions)),
        Key::StageName => stage.map_or_else(String::new, |stage| stage.name.clone()),
        Key::StageInstructions => stage.map_or_else(String::new, |stage| stage.instructions.clone()),
        Key::ExitRule => stage.and_then(|stage| stage.exit_rule.clone()).unwrap_or_default(),
        Key::MaxVisits => number(stage.and_then(|stage| stage.max_visits)),
        Key::TimeoutSeconds => number(execution.and_then(|execution| execution.timeout_ms).map(|ms| ms / 1000)),
        Key::Retries => number(execution.and_then(|execution| execution.retries)),
        Key::Cwd => execution.and_then(|execution| execution.cwd.clone()).unwrap_or_default(),
        Key::RetryDelaySeconds => number(execution.and_then(|execution| execution.retry_delay_ms).map(|ms| ms / 1000)),
        Key::Before => hooks::commands_text(execution.and_then(|execution| execution.before.as_ref())),
        Key::Commands => hooks::commands_text(execution.and_then(|execution| execution.commands.as_ref())),
        Key::After => hooks::commands_text(execution.and_then(|execution| execution.after.as_ref())),
        Key::Gates => hooks::gates_text(execution.and_then(|execution| execution.gates.as_ref())),
        Key::MaxConcurrentRuns => number(scheduler.and_then(|scheduler| scheduler.max_concurrent_runs)),
        Key::PollSeconds => number(scheduler.and_then(|scheduler| scheduler.poll_interval_ms).map(|ms| ms / 1000)),
    }
}

/// Writes `text` for `key`. Workflow and stage keys need their targets; config keys don't.
pub fn apply(key: Key, text: &str, config: &mut ProjectConfig, workflow: usize, stage_id: Option<&str>) {
    match key {
        Key::MaxConcurrentRuns => {
            let scheduler = config.scheduler.get_or_insert_with(SchedulerConfig::default);
            scheduler.max_concurrent_runs = parse(text, scheduler.max_concurrent_runs);
        }
        Key::PollSeconds => {
            let scheduler = config.scheduler.get_or_insert_with(SchedulerConfig::default);
            scheduler.poll_interval_ms = parse::<u64>(text, scheduler.poll_interval_ms.map(|ms| ms / 1000)).map(|s| s * 1000);
        }
        _ => {
            let Some(workflow) = config.workflows.get_mut(workflow) else { return };
            apply_workflow(key, text, workflow, stage_id);
        }
    }
}

fn apply_workflow(key: Key, text: &str, workflow: &mut WorkflowDefinition, stage_id: Option<&str>) {
    match key {
        Key::WorkflowName => workflow.name = text.into(),
        Key::WorkflowInstructions => workflow.instructions = text.into(),
        Key::MaxTransitions => {
            let current = graph::normalized(workflow).max_transitions;
            graph::set_max_transitions(workflow, parse(text, current));
        }
        _ => {
            let Some(stage) = workflow.stages.iter_mut().find(|stage| Some(stage.id.as_str()) == stage_id) else { return };
            apply_stage(key, text, stage);
        }
    }
}

fn apply_stage(key: Key, text: &str, stage: &mut StageDefinition) {
    let prefix = stage.id.clone();
    match key {
        Key::StageName => stage.name = text.into(),
        Key::StageInstructions => stage.instructions = text.into(),
        Key::ExitRule => stage.exit_rule = (!text.trim().is_empty()).then(|| text.into()),
        Key::MaxVisits => stage.max_visits = parse(text, stage.max_visits),
        _ => {
            let execution = stage.execution.get_or_insert_with(StageExecution::default);
            apply_execution(key, text, execution, &prefix);
            if *execution == StageExecution::default() { stage.execution = None }
        }
    }
}

fn apply_execution(key: Key, text: &str, execution: &mut StageExecution, prefix: &str) {
    match key {
        Key::Retries => execution.retries = parse(text, execution.retries),
        Key::Cwd => execution.cwd = (!text.trim().is_empty()).then(|| text.trim().to_string()),
        Key::RetryDelaySeconds => execution.retry_delay_ms = parse::<u64>(text, execution.retry_delay_ms.map(|ms| ms / 1000)).map(|s| s * 1000),
        Key::TimeoutSeconds => execution.timeout_ms = parse::<u64>(text, execution.timeout_ms.map(|ms| ms / 1000)).map(|s| s * 1000),
        Key::Before => execution.before = hooks::parse_commands(text, execution.before.as_ref(), &format!("{prefix}-before")),
        Key::Commands => execution.commands = hooks::parse_commands(text, execution.commands.as_ref(), &format!("{prefix}-command")),
        Key::After => execution.after = hooks::parse_commands(text, execution.after.as_ref(), &format!("{prefix}-after")),
        Key::Gates => execution.gates = hooks::parse_gates(text, execution.gates.as_ref(), &format!("{prefix}-gate")),
        _ => {}
    }
}
