//! "Can return to" picker: a searchable multi-select dropdown (Kit `Combobox`) over the
//! workflow's stages. The trigger shows a one-line summary; each change adds or removes return
//! routes through the graph rules, so a refused return shows the usual reason.

use gpui_kit::component::combobox::{Combobox, ComboboxEvent, ComboboxState};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::*;
use protocol::{StageOutcome, WorkflowDefinition};

use super::{FactoryPane, graph};
use crate::components::Typography as _;
use crate::theme;

#[derive(Clone, PartialEq)]
pub struct ReturnItem {
    id: String,
    name: SharedString,
}

impl SearchableListItem for ReturnItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &String {
        &self.id
    }
}

pub struct ReturnPicker {
    stage_id: String,
    items: Vec<ReturnItem>,
    state: Entity<ComboboxState<Vec<ReturnItem>>>,
    _subscription: Subscription,
}

/// Same stages regardless of order (the dropdown lists choices in click order).
fn same_members(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|id| b.contains(id))
}

/// Current return targets of a stage, in stage order.
fn targets(workflow: &WorkflowDefinition, stage_id: &str) -> Vec<String> {
    let edges = graph::normalized(workflow).edges;
    workflow.stages.iter().map(|stage| stage.id.clone())
        .filter(|id| edges.iter().any(|edge| edge.source_stage_id == stage_id && edge.on == StageOutcome::Return && &edge.target_stage_id == id))
        .collect()
}

fn items(workflow: &WorkflowDefinition, stage_id: &str) -> Vec<ReturnItem> {
    workflow.stages.iter().map(|stage| ReturnItem {
        id: stage.id.clone(),
        name: if stage.id == stage_id { format!("{} (repeat)", stage.name).into() } else { stage.name.clone().into() },
    }).collect()
}

impl FactoryPane {
    /// Keeps the picker for the selected stage in step with the config (stage list, names and
    /// targets, including after undo). Rebuilt only when the stage or its options change.
    pub(super) fn prepare_returns(&mut self, workflow: &WorkflowDefinition, stage_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (options, selected) = (items(workflow, stage_id), targets(workflow, stage_id));
        if let Some(picker) = &self.returns && picker.stage_id == stage_id && picker.items == options {
            let state = picker.state.clone();
            // Resync only on a real difference (undo, a refused route): resyncing clears the search.
            if !same_members(&state.read(cx).selected_values(), &selected) {
                state.update(cx, |state, cx| state.set_selected_values(&selected, window, cx));
            }
            return;
        }
        let indices = selected.iter().filter_map(|id| options.iter().position(|item| &item.id == id)).map(gpui_kit::component::IndexPath::new).collect();
        let state = cx.new(|cx| ComboboxState::new(options.clone(), indices, window, cx).multiple(true).searchable(true));
        let source = stage_id.to_string();
        let subscription = cx.subscribe_in(&state, window, move |this, _, event: &ComboboxEvent<Vec<ReturnItem>>, _, cx| {
            if let ComboboxEvent::Change(values) = event { this.set_returns(&source, values.clone(), cx) }
        });
        self.returns = Some(ReturnPicker { stage_id: stage_id.into(), items: options, state, _subscription: subscription });
    }

    /// Adds and removes return routes so the stage returns to exactly `wanted`.
    fn set_returns(&mut self, source: &str, wanted: Vec<String>, cx: &mut Context<Self>) {
        let Some(mut candidate) = self.config_value(cx).and_then(|config| self.current(&config).cloned()) else { return };
        let current = targets(&candidate, source);
        if same_members(&current, &wanted) { return }
        let removed: Vec<String> = current.iter().filter(|id| !wanted.contains(id)).map(|id| graph::edge_id(source, &StageOutcome::Return, id)).collect();
        graph::remove_edges(&mut candidate, &removed);
        for target in wanted.iter().filter(|id| !current.contains(id)) {
            if let Err(reason) = graph::connect(&mut candidate, source, target, StageOutcome::Return) {
                // Nothing changes; the picker resyncs to the saved targets on the next render.
                self.reject(reason, cx);
                cx.notify();
                return;
            }
        }
        self.edit_workflow(cx, move |workflow| *workflow = candidate);
    }

    pub(super) fn render_returns(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme::tokens(cx);
        let Some(picker) = &self.returns else { return div().into_any_element() };
        let names: Vec<SharedString> = picker.state.read(cx).selection().iter().map(|(_, item)| item.name.clone()).collect();
        let summary = match names.as_slice() {
            [] => "None".to_string(),
            [one] => one.to_string(),
            [first, second] => format!("{first}, {second}"),
            [first, rest @ ..] => format!("{first} +{}", rest.len()),
        };
        div().flex().flex_col().gap_1()
            .child(div().t_label().text_color(t.text_secondary).child("Can return to"))
            .child(Combobox::new(&picker.state).menu_max_h(px(240.)).cleanable(true)
                .search_placeholder("Find a stage")
                .render_trigger(move |_, _, _| div().flex_1().min_w_0().truncate().child(summary.clone())))
            .child(div().t_caption().text_color(t.text_secondary).child("Each target is a rework loop; the run names which one it takes."))
            .into_any_element()
    }
}
