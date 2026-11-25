import { TASK_STATUSES, type TaskStatus } from "@karabiner/contracts";

/** A unique explicit push mapping is also a lossless read mapping. */
export function mappedStatus(
	id: string,
	pull?: Record<string, TaskStatus>,
	push?: Partial<Record<TaskStatus, string>>,
): TaskStatus | undefined {
	if (pull?.[id]) return pull[id];
	const matches = TASK_STATUSES.filter((status) => push?.[status] === id);
	return matches.length === 1 ? matches[0] : undefined;
}
