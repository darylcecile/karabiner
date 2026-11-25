import { readFile, realpath, stat } from "node:fs/promises";
import { isAbsolute, resolve } from "node:path";
import {
	compileFactory,
	exportFactory,
	type FactoryBundle,
	type FactoryDefinition,
	type InstructionDefinition,
	type Project,
	type ProjectConfig,
	type RunSnapshot,
	type Task,
} from "@karabiner/contracts";
import { HttpError, invalid } from "./errors.ts";
import { configInput, object } from "./validation.ts";

async function resolveSource(project: Project, source: InstructionDefinition): Promise<void> {
	if (!source.path) return;

	if (!isAbsolute(source.path) && !project.rootPath)
		invalid(`Source ${source.name} needs a project rootPath for its relative path`);

	const path = isAbsolute(source.path) ? source.path : resolve(project.rootPath!, source.path);

	try {
		const info = await stat(path);

		if (!info.isFile() || info.size > 65_536)
			invalid(`Source ${source.name} must be a file of at most 65536 bytes`);

		source.content = await readFile(path, "utf8");
	} catch (error) {
		if (error instanceof HttpError) throw error;

		throw new HttpError(
			400,
			"source_unavailable",
			`Could not read configured source ${source.name} at ${source.path}`,
		);
	}
}

/** File reads are explicit service inputs; the compiler itself stays deterministic. */
export async function resolveConfigSources(
	project: Project,
	value: unknown,
): Promise<ProjectConfig> {
	if (project.rootPath && !isAbsolute(project.rootPath))
		invalid("Project rootPath must be an absolute path for factory compilation");

	const config = structuredClone(configInput(value));

	await Promise.all(
		[...config.instructions, ...config.skills].map((source) => resolveSource(project, source)),
	);

	return config;
}
export async function compileProjectFactory(
	project: Project,
	config: unknown,
): Promise<FactoryDefinition> {
	return compileFactory(await resolveConfigSources(project, config));
}
export async function exportProjectFactory(
	project: Project,
	config: unknown,
): Promise<FactoryBundle> {
	return exportFactory(await resolveConfigSources(project, config));
}
export async function createSnapshot(
	project: Project,
	task: Task,
	value: ProjectConfig,
	workflowId?: string,
): Promise<RunSnapshot> {
	const config = await resolveConfigSources(project, value);
	const factory = compileFactory(config);

	const selected =
		workflowId ??
		task.workflowId ??
		(factory.workflows.length === 1 ? factory.workflows[0]!.id : undefined);

	if (!selected || !factory.workflows.some((workflow) => workflow.id === selected))
		invalid("Select a workflow in this project's configuration before starting a run");

	const capturedProject = structuredClone(project);

	if (capturedProject.rootPath) {
		try {
			capturedProject.rootPath = await realpath(capturedProject.rootPath);
		} catch {
			invalid("The project's rootPath is not accessible");
		}
	}

	return {
		project: capturedProject,
		task: { ...structuredClone(task), workflowId: selected },
		config,
		factory,
	};
}
export function importFactoryConfig(value: unknown): ProjectConfig {
	const input = object(value);
	const bundle = object(input.bundle, "bundle");

	if (bundle.format !== "karabiner-factory" || bundle.version !== 1)
		invalid("Expected a karabiner-factory version 1 bundle");

	const config = configInput({
		...object(bundle.config, "bundle.config"),
		revision: input.revision,
	});

	compileFactory(config);
	return config;
}
