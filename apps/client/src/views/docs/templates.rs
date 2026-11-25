//! Starter structures offered by "New doc". Plain markdown; every block stays editable.

pub struct DocTemplate {
    pub name: &'static str,
    pub title: &'static str,
    pub markdown: &'static str,
}

pub const TEMPLATES: [DocTemplate; 6] = [
    DocTemplate {
        name: "Blank document",
        title: "Untitled",
        markdown: "",
    },
    DocTemplate {
        name: "Product requirements",
        title: "Product requirements",
        markdown: "## Problem\n\nWhat is broken or missing, and for whom?\n\n## Goals\n\n- Goal one\n\n## Non-goals\n\n- Out of scope\n\n## Requirements\n\n- [ ] First requirement\n\n## Success measures\n\n- How we will know it worked\n\n## Open questions\n\n- What is still unknown?\n",
    },
    DocTemplate {
        name: "Architecture decision",
        title: "ADR: ",
        markdown: "**Status:** Proposed\n\n## Context\n\nWhat forces are at play?\n\n## Decision\n\nWhat are we doing?\n\n## Alternatives considered\n\n- Option and why not\n\n## Consequences\n\nWhat becomes easier or harder?\n",
    },
    DocTemplate {
        name: "Technical design",
        title: "Design: ",
        markdown: "## Summary\n\n## Motivation\n\n## Design\n\n## Risks\n\n- Risk and mitigation\n\n## Rollout\n\n1. First step\n",
    },
    DocTemplate {
        name: "Meeting notes",
        title: "Meeting notes",
        markdown: "**Attendees:** Names\n\n## Notes\n\n- Discussion point\n\n## Decisions\n\n- Decision made\n\n## Follow-ups\n\n- [ ] Owner: action\n",
    },
    DocTemplate {
        name: "Runbook",
        title: "Runbook: ",
        markdown: "## When to use\n\n## Steps\n\n1. First step\n\n## Verify\n\n- [ ] Check the result\n\n## Rollback\n\n1. Undo step\n",
    },
];
