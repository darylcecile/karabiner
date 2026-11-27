# Guided workflow and stage configuration

Status: implemented in source, 2026-11-27; native verification pending. Task: [2293](../tasks/2293-guided-workflow-config.md).

Related: [native factory canvas](native-factory-canvas.md).

## Decision

The factory inspector (`apps/client/src/views/workflow/factory/inspector.rs`) shows the common settings first and keeps technical options one click away:

- **Workflow:** Name, Instructions, Starts at and Instruction files are always visible. *Run limits* is folded. *Project settings* (Environment, Scheduler) has the note "Shared by every workflow in this project."
- **Stage:** Name, Done when and Agents are always visible, followed by Routes. *Instructions* starts open. *Commands and gates*, *Environment* and *Limits* start folded, and each shows a summary of what it holds ("None", "Project default", "Default", "2 commands, 1 gate", "5 visits · 10 min timeout").
- Groups are separated by 24 pt of space and fields by 12 pt, with no rules or frames. Group titles share the fields' left edge, and the chevron sits at the trailing edge. Dropdowns are full width; number fields are 120 pt.

A blank field still means "use the service default". The hints show the service's real fallbacks instead of the old wrong copy:

| Field | Shown default | Source |
|---|---|---|
| Max stage moves | 100 | `packages/contracts/src/factory.ts` |
| Max visits | 3 | `factory.ts` (`maxVisits ?? 3`) |
| Timeout | 10 min for agents, 2 min per command | `runtime/stage-runner.ts` |
| Retries / delay | 0 / 0 | `stage-runner.ts` |
| Runs at once / Check every | 1 / 1 s | `runtime/scheduler.ts` |
| Pick up tasks in | To do | `scheduler.ts` (`statuses ?? ["todo"]`) |

Runtime values are unchanged. Saved values are never overwritten. Clearing a field (including Done when) removes the override.

A new workflow starts with one unassigned stage, so it has an entry point and something to configure. Onboarding now preselects the *Software delivery* template (falling back to the first template), and *Blank workspace* is still listed. That template's description no longer says execution isn't connected.

## Reason

The user found the earlier inspector cramped and poorly grouped. The causes were a redundant Basics group, chevrons that put titles out of line with their fields, uniform spacing, project-wide settings that looked like workflow settings, too many borders, and several wrong default hints. The user also asked for sensible defaults that non-technical users can start from, while keeping every technical option available.

## Not done

- The scheduler stays project-wide. Making it per workflow is a product decision.

Group headers are Kit ghost buttons, so Tab reaches them, Enter or Space toggles them, and they show the focus ring.
