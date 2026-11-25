import type { ProjectConfig, Template } from "@karabiner/contracts";

export function emptyConfig(): ProjectConfig {
	return {
		revision: 0,
		instructions: [],
		skills: [],
		agents: [],
		workflows: [],
		executionTarget: "host",
	};
}
const delivery = emptyConfig();

delivery.agents = [
	{
		id: "planner",
		name: "Planner",
		role: "Clarify and plan",
		harness: "copilot",
		model: "",
		instructions: "Clarify the outcome and record a plan before implementation.",
		skillIds: [],
		mode: "interactive",
	},
	{
		id: "builder",
		name: "Builder",
		role: "Implement",
		harness: "copilot",
		model: "",
		instructions: "Implement the agreed work using the project's conventions.",
		skillIds: [],
		mode: "autonomous",
	},
	{
		id: "reviewer",
		name: "Reviewer",
		role: "Review",
		harness: "copilot",
		model: "",
		instructions: "Review the changes and report concrete findings with source references.",
		skillIds: [],
		mode: "interactive",
	},
];

delivery.workflows = [
	{
		id: "delivery",
		name: "Software delivery",
		description: "Plan, implement and review a change.",
		instructions: "Preserve the user's intended outcome and verify the result.",
		stages: [
			{
				id: "plan",
				name: "Plan",
				instructions: "Agree on acceptance criteria.",
				agentIds: ["planner"],
				returnToStageIds: [],
				exitRule: "The outcome and acceptance criteria are recorded.",
				status: "todo",
				color: "blue",
			},
			{
				id: "implement",
				name: "Implement",
				instructions: "Make the agreed changes.",
				agentIds: ["builder"],
				returnToStageIds: ["plan"],
				exitRule: "The agreed changes and focused checks are complete.",
				status: "in_progress",
				color: "amber",
			},
			{
				id: "review",
				name: "Review",
				instructions: "Verify the result and explain the changes.",
				agentIds: ["reviewer"],
				returnToStageIds: ["implement"],
				exitRule: "Review findings are addressed and the result is explained.",
				status: "in_review",
				color: "violet",
			},
		],
	},
];
export const templates: Template[] = [
	{
		id: "blank",
		name: "Blank workspace",
		description: "Start with your own instructions, agents and workflows.",
		config: emptyConfig(),
	},
	{
		id: "software-delivery",
		name: "Software delivery",
		description:
			"A configurable plan → implement → review workflow. Execution is not connected.",
		config: delivery,
		documents: [
			{
				title: "Project brief",
				kind: "markdown",
				markdown:
					"# Project brief\n\n## Outcome\n\nWhat are we building, and for whom?\n\n## Acceptance criteria\n\n- [ ] Define a measurable outcome\n\n## Constraints\n\nCapture important decisions here.\n",
				tour: null,
			},
		],
	},
	{
		id: "documentation",
		name: "Documentation",
		description: "A project brief and structured documentation starting point.",
		config: emptyConfig(),
		documents: [
			{
				title: "Getting started",
				kind: "markdown",
				markdown:
					"# Getting started\n\nExplain the project in a sentence.\n\n## Setup\n\n```sh\n# Add setup commands\n```\n\n## How it works\n\nDescribe the important concepts.\n\n## References\n\n- Add links to useful resources\n",
				tour: null,
			},
		],
	},
];
