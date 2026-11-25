import { mkdir, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import type { FactoryBundle, FactoryDefinition, FactoryValidation, ProjectConfig } from "@karabiner/contracts";
import { argument, flag, jsonFile, maxArgs, required, revision, validateFlags, type Flags } from "./arguments.ts";
import { Client, ClientError } from "./client.ts";

const factoryFlags: Record<string, readonly string[]> = {
  show: ["project"], get: ["project"], workflows: ["project"],
  validate: ["project", "file"], compile: ["project", "file"],
  export: ["project", "output", "directory"], import: ["project", "file", "revision"],
};

async function saveBundle(bundle: FactoryBundle, flags: Flags): Promise<unknown> {
  const output = flag(flags, "output"), directory = flag(flags, "directory");
  const content = JSON.stringify(bundle, null, 2) + "\n";
  if (output !== undefined) {
    const path = resolve(required(output, "--output FILE"));
    await writeFile(path, content, { flag: "wx" });
    return { path, factoryId: bundle.definition.id };
  }
  if (directory === undefined) return bundle;
  const destination = resolve(required(directory, "--directory DIR"));
  const files = [{ path: "bundle.json", content }, ...bundle.files];
  const paths = new Set<string>();
  for (const file of files) {
    const path = resolve(destination, file.path), local = relative(destination, path);
    if (!local || isAbsolute(file.path) || local === ".." || local.startsWith("../") || paths.has(path)) throw new ClientError("invalid_response", `Invalid or duplicate exported file path: ${file.path}`);
    paths.add(path);
  }
  // A fresh directory prevents overwriting an existing project or following its symlinks.
  await mkdir(destination);
  for (const file of files) {
    const path = resolve(destination, file.path);
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, file.content, { flag: "wx" });
  }
  return { directory: destination, files: [...paths], factoryId: bundle.definition.id };
}

export async function factoryCommand(client: Client, args: string[], flags: Flags): Promise<unknown> {
  const [action = "show"] = args, allowed = factoryFlags[action];
  if (!allowed) throw new ClientError("usage", `Unknown factory action: ${action}`);
  maxArgs(args, 2, `factory ${action}`);
  validateFlags(`factory ${action}`, flags, allowed);
  const projectId = argument(args, 1, flags, "project", "PROJECT"), path = `/v1/projects/${encodeURIComponent(projectId)}/factory`;
  if (flag(flags, "output") !== undefined && flag(flags, "directory") !== undefined) throw new ClientError("usage", "Choose one export destination: --output FILE or --directory DIR");
  if (action === "import") {
    const configRevision = revision(flags);
    if (configRevision === undefined) throw new ClientError("usage", "--revision is required; use the revision from config get PROJECT");
    const bundle = await jsonFile(required(flag(flags, "file"), "--file bundle.json"));
    await client.connect();
    return client.request<ProjectConfig>("POST", `${path}/import`, { revision: configRevision, bundle });
  }
  const input = flag(flags, "file");
  const config = input === undefined ? undefined : await jsonFile(input);
  await client.connect();
  if (action === "validate") {
    const result = await client.request<FactoryValidation>("POST", `${path}/validate`, { config });
    if (!result.valid) process.exitCode = 1;
    return result;
  }
  if (action === "compile") return client.request<FactoryDefinition>("POST", `${path}/compile`, { config });
  if (action === "export") return saveBundle(await client.request<FactoryBundle>("GET", `${path}/export`), flags);
  const definition = await client.request<FactoryDefinition>("GET", path);
  return action === "workflows" ? definition.workflows : definition;
}
