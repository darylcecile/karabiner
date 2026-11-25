export class HttpError extends Error {
	constructor(
		readonly status: number,
		readonly code: string,
		message: string,
	) {
		super(message);
	}
}
export function invalid(message: string): never {
	throw new HttpError(400, "invalid_request", message);
}
export function missing(resource: string): never {
	throw new HttpError(404, "not_found", `${resource} was not found`);
}
export function conflict(message: string): never {
	throw new HttpError(409, "conflict", message);
}
