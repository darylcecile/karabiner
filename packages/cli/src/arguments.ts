import { ClientError } from "./client.ts";

export type Flags = Record<string, string | boolean | undefined>;
export function flag(flags: Flags, name: string): string | undefined { const value = flags[name]; return typeof value === "string" ? value : undefined; }
export function required(value: string | undefined, label: string): string { if (!value) throw new ClientError("usage", `${label} is required. Run --help for usage.`); return value; }
export async function file(path: string): Promise<string> { return path === "-" ? await Bun.stdin.text() : await Bun.file(path).text(); }
export async function jsonFile(path: string): Promise<Record<string, unknown>> {
  const value: unknown = JSON.parse(await file(path));
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new ClientError("usage", "JSON input must be an object");
  return value as Record<string, unknown>;
}
export function query(flags: Flags, mappings: readonly (readonly [string, string])[] = [["project", "projectId"], ["task", "taskId"], ["limit", "limit"]]): string {
  const params = new URLSearchParams();
  for (const [option, key] of mappings) {
    const value = flag(flags, option);
    if (value !== undefined) params.set(key, value);
  }
  return params.size ? `?${params}` : "";
}
export function validateFlags(command: string, flags: Flags, allowed: readonly string[]): void {
  for (const name of Object.keys(flags)) {
    if (name !== "url" && name !== "json" && !allowed.includes(name)) throw new ClientError("usage", `--${name} is not valid for ${command}`);
  }
}
export function revision(flags: Flags): number | undefined {
  const value = flag(flags, "revision");
  if (value === undefined) return undefined;
  const number = Number(value);
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(number)) throw new ClientError("usage", "--revision must be a non-negative integer");
  return number;
}
export function argument(args: string[], index: number, flags: Flags, name: string, label: string): string {
  const positional = args[index], option = flag(flags, name);
  if (positional !== undefined && option !== undefined && positional !== option) throw new ClientError("usage", `${label} and --${name} must identify the same value`);
  return required(option ?? positional, label);
}
export function maxArgs(args: string[], count: number, command: string): void {
  if (args.length > count) throw new ClientError("usage", `Unexpected arguments for ${command}. Run --help for usage.`);
}
