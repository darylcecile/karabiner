import type { AgentDefinition } from "@karabiner/contracts";
import type { Model } from "@strands-agents/sdk";
import { HttpError } from "../errors.ts";

const credentialNames: Record<string, string> = {
	anthropic: "ANTHROPIC_API_KEY",
	openai: "OPENAI_API_KEY",
	google: "GEMINI_API_KEY",
	bedrock: "AWS_BEARER_TOKEN_BEDROCK",
	litellm: "LITELLM_API_KEY",
};

export function strandsReadiness(agent: AgentDefinition) {
	const provider = agent.strands?.provider ?? agent.model.split("/")[0];
	const credentialEnv = agent.strands?.credentialEnv ?? credentialNames[provider ?? ""];

	const required =
		Boolean(agent.strands?.credentialEnv) ||
		["anthropic", "openai", "google"].includes(provider ?? "");

	if (!agent.model.trim())
		return {
			available: false,
			authenticated: null,
			code: "model_missing",
			message: "Configure a Strands model",
		};

	if (required && (!credentialEnv || !process.env[credentialEnv]))
		return {
			available: false,
			authenticated: null,
			code: "credential_missing",
			credentialEnv,
			message: `Set ${credentialEnv} in the service environment; only its name is stored in configuration`,
		};

	return {
		available: true,
		authenticated: null,
		code: "configured",
		credentialEnv,
		message: required
			? "Credential reference is present; provider authentication is unverified"
			: "Provider is configured; endpoint access and authentication are unverified",
	};
}

/** Construct per-agent clients without changing shared process environment or persisting credential values. */
export async function resolveStrandsModel(agent: AgentDefinition): Promise<Model | string> {
	const config = agent.strands;
	if (!config) return agent.model;
	const readiness = strandsReadiness(agent);
	if (!readiness.available) throw new HttpError(400, readiness.code, readiness.message);
	const apiKey = readiness.credentialEnv ? process.env[readiness.credentialEnv] : undefined;

	const modelId = agent.model.startsWith(`${config.provider}/`)
		? agent.model.slice(config.provider.length + 1)
		: agent.model;

	if (config.provider === "bedrock") {
		const { BedrockModel } = await import("@strands-agents/sdk/models/bedrock");

		return new BedrockModel({
			modelId,
			apiKey,
			region: config.region,
			...(config.baseUrl ? { clientConfig: { endpoint: config.baseUrl } } : {}),
		});
	}

	if (config.provider === "anthropic") {
		const { AnthropicModel } = await import("@strands-agents/sdk/models/anthropic");

		return new AnthropicModel({
			modelId,
			apiKey,
			...(config.baseUrl ? { clientConfig: { baseURL: config.baseUrl } } : {}),
		});
	}

	if (config.provider === "google") {
		const { GoogleModel } = await import("@strands-agents/sdk/models/google");

		return new GoogleModel({
			modelId,
			apiKey,
			...(config.baseUrl
				? { clientConfig: { httpOptions: { baseUrl: config.baseUrl } } }
				: {}),
		});
	}

	const { OpenAIModel } = await import("@strands-agents/sdk/models/openai");

	const baseURL =
		config.baseUrl ??
		(config.provider === "ollama"
			? "http://127.0.0.1:11434/v1"
			: config.provider === "litellm"
				? "http://127.0.0.1:4000/v1"
				: undefined);

	return new OpenAIModel({
		modelId,
		api: config.provider === "openai" ? (config.api ?? "responses") : "chat",
		apiKey: apiKey ?? (config.provider === "openai" ? undefined : config.provider),
		...(baseURL ? { clientConfig: { baseURL } } : {}),
	});
}
