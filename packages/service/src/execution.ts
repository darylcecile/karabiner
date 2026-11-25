import type { Run } from "@karabiner/contracts";

/** User-owned extension point for observed execution records, independent from CRUD. */
export interface RunReader {
  list(filter: { taskId?: string }): Promise<Run[]>;
  get(id: string): Promise<Run | null>;
}
export const unimplementedRuns: RunReader = {
  async list() { return []; },
  async get() { return null; },
};
