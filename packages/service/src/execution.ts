import { FactoryRuntime, type RuntimeOptions } from "./runtime/scheduler.ts";

export { FactoryRuntime, type RuntimeOptions } from "./runtime/scheduler.ts";
export type { AdapterRequest, AgentOutcome } from "./runtime/adapter-types.ts";

export function createRuntime(options: RuntimeOptions): FactoryRuntime {
	return new FactoryRuntime(options);
}
