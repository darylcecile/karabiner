import type { CodeTour, Document, Project, Task } from "@karabiner/contracts";
import { Client } from "./client.ts";

/** Explicit sample records. They never pretend to be a completed agent execution. */
export async function seedDemo(client: Client) {
  const project = await client.request<Project>("POST", "/v1/projects", { name: "[Demo] Northstar", description: "Illustrative sample records for exploring Karabiner. Task statuses and commits are examples; no agents have run.", kind: "standalone", templateId: "software-delivery", color: "blue" });
  const tasks: Task[] = [];
  for (const [title, status, priority, description] of [
    ["Design the project workspace", "done", "high", "Document navigation and keyboard interactions. This is a manually authored demo status."],
    ["Add a project switcher", "in_review", "high", "Keep open work visible while switching between projects."],
    ["Persist the document outline", "in_progress", "normal", "Retain the selected chapter when reopening a document."],
    ["Explore keyboard shortcuts", "todo", "normal", "Map the most frequent actions to discoverable shortcuts."],
    ["Connect an execution harness", "blocked", "urgent", "Execution adapters are intentionally left for implementation."],
  ]) tasks.push(await client.request<Task>("POST", "/v1/tasks", { projectId: project.id, title, status, priority, description, workflowId: "delivery" }));
  const tour: CodeTour = {
    overview: "An illustrative tour of a small project-switcher change. These patches and commit IDs are sample content, not repository observations.", risk: "low", impact: "medium",
    chapters: [
      { id: "selection", title: "One owner for project selection", summary: "Keep the selected project in workspace state so the sidebar and content agree.", commitShas: ["demo-a1"], filePaths: ["src/workspace.ts"], blocks: [
        { id: "intro", kind: "markdown", text: "The workspace owns **selection**. Views read that value instead of keeping separate copies." },
        { id: "flow", kind: "diagram", title: "How selection flows", format: "mermaid", source: "flowchart LR\n  Sidebar --> Workspace\n  Workspace --> Board\n  Workspace --> Documents" },
        { id: "change", kind: "diff", title: "Remember project selection", path: "src/workspace.ts", baseRevision: "demo-base", headRevision: "demo-a1", patch: "@@ -1,3 +1,5 @@\n export interface Workspace {\n   tabs: Tab[];\n+  selectedProjectId: string | null;\n+  activeTabId: string | null;\n }" },
      ] },
      { id: "restore", title: "Restore the active context", summary: "Reopen the selected project and its saved tabs together.", commitShas: ["demo-b2"], filePaths: ["src/restore.ts"], blocks: [
        { id: "code", kind: "code", title: "Restore workspace", language: "typescript", source: { path: "src/restore.ts", revision: "demo-b2", startLine: 1, endLine: 4 }, code: "const workspace = await service.workspace();\nsetTabs(workspace.tabs);\nsetActiveTab(workspace.activeTabId);\nsetProject(workspace.selectedProjectId);" },
      ] },
    ],
    commits: [
      { sha: "demo-a1", message: "Store project selection", author: "Demo author", createdAt: new Date().toISOString(), files: [{ path: "src/workspace.ts", status: "modified", additions: 2, deletions: 0, patch: "@@ -1,1 +1,2 @@\n tabs: Tab[];\n+selectedProjectId: string | null;" }] },
      { sha: "demo-b2", message: "Restore workspace context", author: "Demo author", createdAt: new Date().toISOString(), files: [{ path: "src/restore.ts", status: "added", additions: 4, deletions: 0, patch: "+const workspace = await service.workspace();" }] },
    ], commitGroups: [{ id: "workspace", title: "Workspace persistence", commitShas: ["demo-a1", "demo-b2"] }],
    review: { reviewedChapterIds: [], reviewedFilePaths: [], decision: "pending", summary: "" },
  };
  const document = await client.request<Document>("POST", "/v1/documents", { projectId: project.id, taskId: tasks[1]!.id, title: "[Demo] Project switcher code tour", kind: "code_tour", tour });
  await client.request("POST", "/v1/documents", { projectId: project.id, title: "[Demo] Workspace principles", markdown: "# Workspace principles\n\nKeep context close to the work.\n\n## Navigation\n\n- Projects are persistent homes for tasks and documents.\n- Tabs preserve what you are reading.\n- **The service owns data**; the client owns presentation.\n\n> Sample content for exploring the document editor.\n\n| Action | Shortcut |\n| --- | --- |\n| Close tab | ⌘W |\n| Reopen tab | ⌘⇧T |\n\n## Next steps\n\n- [ ] Connect a harness\n- [ ] Choose a workflow\n" });
  await client.request("POST", "/v1/interventions", { projectId: project.id, taskId: tasks[4]!.id, kind: "question", message: "[Demo] Which local harness should this project use? This question is a stored example, not a live agent request." });
  return { demo: true, project, taskIds: tasks.map(task => task.id), tourId: document.id };
}
