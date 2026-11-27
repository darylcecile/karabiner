import type { RunActivityEvent } from "@karabiner/contracts";
import type { AdapterEvent } from "./adapter-types.ts";

const fieldLimit = 32_000;
const activityLimit = 256_000;
const eventLimit = 512;
const truncated = "\n[Content truncated]";
const omitted = "[Earlier activity truncated]";

const privateField =
	/^(?:env|environment|authorization|cookies?|password|secret|private[_-]?key|api[_-]?key|(?:access|refresh)?[_-]?token|credentials)$/i;

function bounded(text: string): string {
	return text.length > fieldLimit
		? text.slice(0, fieldLimit - truncated.length) + truncated
		: text;
}

/** Serialize only explicitly selected tool data, never SDK/session or launch configuration. */
function activityData(value: unknown): string | undefined {
	if (value === undefined) return;
	if (typeof value === "string") return bounded(value);

	try {
		return bounded(
			JSON.stringify(
				value,
				(key, item: unknown) => {
					if (privateField.test(key)) return "[Redacted]";
					if (typeof item === "bigint") return String(item);
					if (item instanceof Error) return { message: item.message };

					return item;
				},
				2,
			) ?? "[No serializable content]",
		);
	} catch {
		return "[Tool data could not be serialized]";
	}
}

/** A small rolling journal. Tool completions update their exact call, not the latest name. */
export class ActivityRecorder {
	readonly events: RunActivityEvent[];
	private appendTo?: RunActivityEvent;
	private appendKind?: AdapterEvent["kind"];
	private messageId?: string;

	constructor(events: RunActivityEvent[] = []) {
		this.events = structuredClone(events);
	}

	add(event: AdapterEvent): void {
		if (event.kind === "tool") {
			this.appendTo = undefined;
			this.tool(event);
		} else {
			const kind =
				event.kind === "reasoning" || event.kind === "error" ? event.kind : "message";

			const text = event.kind === "activity" ? `${event.text}\n` : event.text;
			if (!text) return;
			const last = this.events.at(-1);

			if (
				event.delta !== false &&
				event.kind === this.appendKind &&
				event.messageId === this.messageId &&
				last === this.appendTo &&
				last?.kind === kind
			)
				last.text = bounded((last.text ?? "") + text);
			else {
				this.events.push({ kind, at: new Date().toISOString(), text: bounded(text) });
			}

			this.appendTo = event.delta === false ? undefined : this.events.at(-1);
			this.appendKind = event.kind;
			this.messageId = event.messageId;
		}

		this.trim();
	}

	private tool(event: AdapterEvent): void {
		let entry = event.toolId
			? this.events.find((item) => item.kind === "tool" && item.toolId === event.toolId)
			: undefined;

		if (!entry) {
			entry = { kind: "tool", at: new Date().toISOString(), toolId: event.toolId };
			this.events.push(entry);
		}

		entry.name = bounded(event.name ?? entry.name ?? event.text);
		entry.status = event.status ?? entry.status ?? "running";
		if (event.input != null) entry.input = activityData(event.input);
		if (event.output != null) entry.output = activityData(event.output);

		if (event.outputDelta !== undefined)
			entry.output = bounded((entry.output ?? "") + event.outputDelta);
	}

	private trim(): void {
		let size = this.events.reduce(
			(total, event) => total + JSON.stringify(event).length + 1,
			2,
		);

		let removed = false;

		while (this.events.length > eventLimit - 1 || size > activityLimit - omitted.length - 100) {
			size -= JSON.stringify(this.events.shift()!).length + 1;
			removed = true;
		}

		if (removed && this.events[0]?.text !== omitted)
			this.events.unshift({ kind: "message", text: omitted });
	}
}
