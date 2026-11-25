import { lstat, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, relative } from "node:path";
import type { StoredRun, TourFileChange } from "@karabiner/contracts";
import type { Repository } from "../repository.ts";
import { runProcess } from "./process.ts";

interface Baseline {
	root: string;
	head: string;
	dirty: Record<string, string | null>;
}

const maxFileBytes = 256 * 1024;

async function git(root: string, args: string[]) {
	return runProcess({
		command: "git",
		args: ["--no-optional-locks", ...args],
		cwd: root,
		timeoutMs: 20_000,
	});
}

async function source(root: string, path: string): Promise<string | null> {
	const absolute = resolve(root, path);

	if (relative(root, absolute).startsWith(".."))
		throw new Error("Source path leaves the project");

	try {
		const stat = await lstat(absolute);
		if (!stat.isFile() || stat.size > maxFileBytes) return null;
		const bytes = await readFile(absolute);
		return bytes.includes(0) ? null : bytes.toString("utf8");
	} catch (error) {
		if ((error as NodeJS.ErrnoException).code === "ENOENT") return "";
		throw error;
	}
}

async function changedPaths(root: string, head: string): Promise<string[]> {
	const [tracked, untracked] = await Promise.all([
		git(root, ["diff", "--name-only", "-z", head, "--"]),
		git(root, ["ls-files", "--others", "--exclude-standard", "-z"]),
	]);

	if (tracked.code || untracked.code) throw new Error("Could not inspect repository changes");
	return [...new Set(`${tracked.stdout}\0${untracked.stdout}`.split("\0").filter(Boolean))];
}

export async function captureBaseline(rootPath: string): Promise<Baseline | null> {
	const root = await git(rootPath, ["rev-parse", "--show-toplevel"]);
	if (root.code) return null;
	const head = await git(root.stdout.trim(), ["rev-parse", "HEAD"]);
	if (head.code) return null;
	const baseline: Baseline = { root: root.stdout.trim(), head: head.stdout.trim(), dirty: {} };

	for (const path of await changedPaths(baseline.root, baseline.head))
		baseline.dirty[path] = await source(baseline.root, path);

	return baseline;
}

async function beforeSource(baseline: Baseline, path: string) {
	if (Object.hasOwn(baseline.dirty, path)) return baseline.dirty[path] ?? null;
	const result = await git(baseline.root, ["show", `${baseline.head}:${path}`]);
	if (result.code) return "";

	return Buffer.byteLength(result.stdout) > maxFileBytes || result.stdout.includes("\0")
		? null
		: result.stdout;
}

async function diffFile(path: string, before: string, after: string): Promise<TourFileChange> {
	const directory = await mkdtemp(join(tmpdir(), "karabiner-diff-"));

	try {
		await Promise.all([
			writeFile(join(directory, "before"), before),
			writeFile(join(directory, "after"), after),
		]);

		const result = await git(directory, [
			"diff",
			"--no-index",
			"--no-ext-diff",
			"--",
			"before",
			"after",
		]);

		if (result.code > 1) throw new Error(result.stderr || "Could not build source diff");

		const patch = result.stdout
			.replace(/^diff --git a\/before b\/after/m, `diff --git a/${path} b/${path}`)
			.replace(/^--- a\/before/m, `--- a/${path}`)
			.replace(/^\+\+\+ b\/after/m, `+++ b/${path}`);

		const lines = patch.split("\n");

		return {
			path,
			status: before === "" ? "added" : after === "" ? "deleted" : "modified",
			patch,
			additions: lines.filter((line) => line.startsWith("+") && !line.startsWith("+++"))
				.length,
			deletions: lines.filter((line) => line.startsWith("-") && !line.startsWith("---"))
				.length,
		};
	} finally {
		await rm(directory, { recursive: true, force: true });
	}
}

export async function createRunDocuments(
	repository: Repository,
	run: StoredRun,
	baseline: Baseline | null,
	publish?: (artifact: StoredRun["artifacts"][number]) => void,
): Promise<StoredRun["artifacts"]> {
	const stages = (run.stages ?? [])
		.map(
			(stage) =>
				`## ${stage.title} · visit ${stage.visit ?? 1}\n\n${stage.steps.map((step) => `### ${step.title} — ${step.status}\n\n${step.output ?? step.activity ?? ""}`).join("\n\n")}`,
		)
		.join("\n\n");

	const markdown = `# ${run.snapshot.task.title}\n\nRun: \`${run.id}\` · Factory: \`${run.snapshot.factory.id}\`\n\nStatus: **${run.state}**\n\n${run.output ?? "No final agent summary."}\n\n${stages}\n\n${run.errors.map((error) => `- ${error.message}`).join("\n")}\n`;

	const summary = repository.createDocument({
		projectId: run.projectId,
		taskId: run.taskId,
		title: `Run · ${run.snapshot.task.title}`,
		kind: "markdown",
		markdown,
	});

	const artifacts: StoredRun["artifacts"] = [
		{ id: crypto.randomUUID(), kind: "document", name: summary.title, documentId: summary.id },
	];

	publish?.(artifacts[0]!);
	if (!baseline) return artifacts;

	const paths = new Set([
		...(await changedPaths(baseline.root, baseline.head)),
		...Object.keys(baseline.dirty),
	]);

	const files: TourFileChange[] = [];
	const omitted: string[] = [];

	for (const path of paths) {
		const [before, after] = await Promise.all([
			beforeSource(baseline, path),
			source(baseline.root, path),
		]);

		if (before === null || after === null) {
			omitted.push(path);
			continue;
		}

		if (before !== after) files.push(await diffFile(path, before, after));
	}

	if (!files.length && !omitted.length) return artifacts;
	const head = await git(baseline.root, ["rev-parse", "HEAD"]);
	const overview = `Changes observed during run ${run.id}, compared with the working tree captured at start (HEAD ${baseline.head}). Existing dirty content is included in the baseline.${omitted.length ? ` Binary, nonregular or large files omitted: ${omitted.join(", ")}.` : ""}`;

	const tour = repository.createDocument({
		projectId: run.projectId,
		taskId: run.taskId,
		title: `Changes · ${run.snapshot.task.title}`,
		kind: "code_tour",
		markdown: overview,
		tour: {
			overview,
			chapters: files.map((file) => ({
				id: crypto.randomUUID(),
				title: file.path,
				summary: `${file.additions} additions, ${file.deletions} deletions observed during this run.`,
				filePaths: [file.path],
				blocks: [
					{
						id: crypto.randomUUID(),
						kind: "diff",
						title: file.path,
						path: file.path,
						baseRevision: baseline.head,
						headRevision: head.code ? null : head.stdout.trim(),
						patch: file.patch,
					},
				],
			})),
			review: {
				reviewedChapterIds: [],
				reviewedFilePaths: [],
				decision: "pending",
				summary: "",
			},
		},
	});

	artifacts.push({
		id: crypto.randomUUID(),
		kind: "code_tour",
		name: tour.title,
		documentId: tour.id,
	});

	publish?.(artifacts.at(-1)!);
	return artifacts;
}
