import type {
	InteractionField,
	InteractionResponse,
	Json,
	RunQuestion,
} from "@karabiner/contracts";
import { invalid } from "../errors.ts";

function reject(field: InteractionField, message: string): never {
	return invalid(`Response field ${field.id} ${message}`);
}

function membership(field: InteractionField, values: string[]): void {
	if (field.allowOther || field.options === undefined) return;

	if (values.some((value) => !field.options!.some((option) => option.id === value)))
		reject(field, "must use its supplied options");
}

function numeric(field: InteractionField, raw: unknown): number {
	if (typeof raw !== "number" && typeof raw !== "string") reject(field, "must be a number");
	const value = typeof raw === "string" ? Number(raw.trim()) : raw;
	if (!Number.isFinite(value)) reject(field, "must be a finite number");

	if (field.type === "integer" && !Number.isSafeInteger(value))
		reject(field, "must be a safely representable integer");

	if (field.minimum !== undefined && value < field.minimum)
		reject(field, `must be at least ${field.minimum}`);

	if (field.maximum !== undefined && value > field.maximum)
		reject(field, `must be at most ${field.maximum}`);

	membership(field, [String(value)]);
	return value;
}

function boolean(field: InteractionField, raw: unknown): boolean {
	const value = typeof raw === "string" ? raw.trim().toLowerCase() : raw;
	let result: boolean;

	if (value === true || value === "true") result = true;
	else if (value === false || value === "false") result = false;
	else return reject(field, "must be true or false");

	membership(field, [String(result)]);
	return result;
}

function selection(field: InteractionField, raw: unknown): string[] {
	if (!Array.isArray(raw) || raw.some((value) => typeof value !== "string"))
		reject(field, "must be an array of option strings");

	if (field.minItems !== undefined && raw.length < field.minItems)
		reject(field, `requires at least ${field.minItems} selections`);

	if (field.maxItems !== undefined && raw.length > field.maxItems)
		reject(field, `allows at most ${field.maxItems} selections`);

	membership(field, raw);
	return raw;
}

function fieldValue(field: InteractionField, raw: unknown): Json | undefined {
	const empty =
		raw === undefined ||
		(typeof raw === "string" && !raw.trim()) ||
		(Array.isArray(raw) && !raw.length);

	if (field.required && empty) reject(field, "is required");

	if (
		raw === undefined ||
		(empty && ["number", "integer", "boolean", "select"].includes(field.type))
	)
		return undefined;

	if (field.type === "number" || field.type === "integer") return numeric(field, raw);
	if (field.type === "boolean") return boolean(field, raw);
	if (field.type === "multiselect") return selection(field, raw);
	if (typeof raw !== "string") reject(field, "must be text");
	const length = [...raw].length;

	if (field.minLength !== undefined && length < field.minLength)
		reject(field, `requires at least ${field.minLength} characters`);

	if (field.maxLength !== undefined && length > field.maxLength)
		reject(field, `allows at most ${field.maxLength} characters`);

	membership(field, [raw]);
	return raw;
}

/** Normalize only the declared primitive form. Other JSON replies retain their original meaning. */
export function normalizeInteractionResponse(
	question: RunQuestion,
	response: InteractionResponse,
): InteractionResponse {
	const effect = question.choices?.find((choice) => choice.id === response.choice)?.effect;
	if (effect === "deny" || effect === "cancel") return { choice: response.choice };
	if (!question.fields) return response;

	const raw = Object.hasOwn(response, "value")
		? response.value
		: (response.answers ??
			(question.fields.length === 1 && response.answer !== undefined
				? { [question.fields[0]!.id]: response.answer }
				: {}));

	if (!raw || typeof raw !== "object" || Array.isArray(raw))
		invalid("A form response must contain an object of field values");

	const fields = new Set(question.fields.map((field) => field.id));

	for (const key of [...Object.keys(raw), ...Object.keys(response.answers ?? {})])
		if (!fields.has(key)) invalid(`Unknown response field ${key}`);

	const values: Record<string, Json> = {};

	for (const field of question.fields) {
		const value = fieldValue(
			field,
			Object.hasOwn(raw, field.id) ? (raw as Record<string, unknown>)[field.id] : undefined,
		);

		if (value !== undefined)
			Object.defineProperty(values, field.id, { value, enumerable: true });
	}

	return { ...response, value: values };
}
