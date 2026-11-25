/** Best-effort invalidations. Reconnect means refetch; this is not a task event log. */
export class Changes {
	private clients = new Set<ReadableStreamDefaultController<Uint8Array>>();
	private encoder = new TextEncoder();
	response(): Response {
		let client: ReadableStreamDefaultController<Uint8Array>;

		const stream = new ReadableStream<Uint8Array>({
			start: (controller) => {
				client = controller;
				this.clients.add(controller);
				controller.enqueue(this.encoder.encode('event: ready\ndata: {"type":"ready"}\n\n'));
			},
			cancel: () => {
				this.clients.delete(client);
			},
		});

		return new Response(stream, {
			headers: { "Content-Type": "text/event-stream", "Cache-Control": "no-cache" },
		});
	}
	changed(path: string): void {
		const bytes = this.encoder.encode(
			`event: change\ndata: ${JSON.stringify({ type: "changed", path })}\n\n`,
		);

		for (const client of this.clients) {
			try {
				if (client.desiredSize !== null && client.desiredSize <= 0) {
					client.close();
					this.clients.delete(client);
				} else client.enqueue(bytes);
			} catch {
				this.clients.delete(client);
			}
		}
	}
	close(): void {
		for (const client of this.clients) {
			try {
				client.close();
			} catch {
				/* Already disconnected. */
			}
		}

		this.clients.clear();
	}
}
