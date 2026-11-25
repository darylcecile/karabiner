/** Controlled executable peer for the released SDK wire protocols. Never contacts a provider. */
import assert from "node:assert/strict";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { spawn } from "node:child_process";

type Frame = Record<string, any>;
const peerName = basename(process.argv[1]!).replace("native-", "");
const provider = peerName === "copilot-protocol3" ? "copilot" : peerName;
const copilotProtocol = peerName === "copilot-protocol3" ? 3 : 4;
mkdirSync(".fixture", { recursive: true });
const log = (event: string, data: unknown = {}) => appendFileSync(".fixture/native-wire.jsonl", `${JSON.stringify({ provider, pid: process.pid, event, data })}\n`);
log("started", { args: process.argv.slice(2), cwd: process.cwd() });

if (provider === "mxc") {
  const config = JSON.parse(readFileSync(3, "utf8"));
  log("fd3", { cwd: config.process.cwd, commandLine: config.process.commandLine });
  const env = Object.fromEntries(config.process.env.map((item: string) => { const index = item.indexOf("="); return [item.slice(0, index), item.slice(index + 1)]; }));
  const child = spawn("/bin/sh", ["-c", config.process.commandLine], { cwd: config.process.cwd, env, stdio: "inherit" });
  process.on("SIGTERM", () => child.kill("SIGTERM"));
  child.on("close", (code) => process.exit(code ?? 1));
  child.on("error", (error) => { log("error", error.message); process.exit(1); });
} else if (provider === "claude" && process.argv.includes("auth")) {
  log("readiness");
  console.log(JSON.stringify({ loggedIn: true, authMethod: "fixture" }));
} else {
  startPeer();
}

function startPeer() {
  let sessionId = "fixture-session", turnId = "fixture-turn", sequence = 0, aborted = false;
  let runRequest: Frame | undefined, firstUuid = "";
  let guidance: string | undefined;
  let guidanceDone!: () => void;
  const gotGuidance = new Promise<void>((resolve) => { guidanceDone = resolve; });
  const pending = new Map<string, (value: any) => void>();
  const outcome = (kind = "complete") => ({ outcome: kind, summary: kind === "blocked" ? "Operator denied the owned fixture effect" : `Observed native approval and ${guidance}`, returnToStageId: null, question: null });
  const send = (frame: Frame) => {
    const text = JSON.stringify(frame);
    process.stdout.write(provider === "copilot" ? `Content-Length: ${Buffer.byteLength(text)}\r\n\r\n${text}` : `${text}\n`);
  };
  const reply = (frame: Frame, result: unknown) => send({ jsonrpc: "2.0", id: frame.id, result });
  const notification = (method: string, params: unknown) => send({ jsonrpc: "2.0", method, params });
  const event = (type: string, data: Frame) => {
    const value = { id: crypto.randomUUID(), timestamp: new Date().toISOString(), parentId: null, type, data };
    notification("session.event", { sessionId, event: value });
    return value;
  };
  const ask = (method: string, params: Frame): Promise<any> => {
    const id = `native-${++sequence}`;
    return new Promise((resolve) => {
      pending.set(id, resolve);
      if (provider === "claude") send({ type: "control_request", request_id: id, request: { subtype: method, ...params } });
      else send({ jsonrpc: "2.0", id, method, params: { sessionId, threadId: sessionId, turnId, ...params } });
    });
  };
  const broadcastAsk = (type: string, data: Frame): Promise<any> => {
    const requestId = `native-${++sequence}`;
    return new Promise((resolve) => { pending.set(requestId, resolve); event(type, { requestId, ...data }); });
  };
  const rememberGuidance = (text: string, id?: string) => {
    guidance = text;
    log("guidance", { text, id });
    if (provider === "copilot") event("user.message", { messageId: `steer-${sequence}`, clientTurnId: id, content: text });
    if (provider === "claude") send({ type: "assistant", uuid: crypto.randomUUID(), session_id: sessionId, parent_tool_use_id: null, user_message_uuid: id, user_message_uuids: [id], message: { id: "fixture-assistant", type: "message", role: "assistant", model: "fixture", content: [{ type: "text", text: "Guidance consumed" }], stop_reason: null, usage: { input_tokens: 1, output_tokens: 1 } } });
    guidanceDone();
  };
  const finish = (kind = "complete") => {
    if (aborted || !runRequest) return;
    const result = outcome(kind);
    log("finished", result);
    if (provider === "claude") send({ type: "result", subtype: "success", is_error: false, session_id: sessionId, uuid: crypto.randomUUID(), result: JSON.stringify(result), structured_output: result, duration_ms: 1, duration_api_ms: 0, num_turns: 1, total_cost_usd: 0, usage: { input_tokens: 2, output_tokens: 1 }, modelUsage: {}, permission_denials: [], user_message_uuid: firstUuid, user_message_uuids: [firstUuid] });
    if (provider === "codex") { notification("item/completed", { threadId: sessionId, item: { id: "final", type: "agentMessage", text: JSON.stringify(result) } }); notification("turn/completed", { threadId: sessionId, turn: { id: turnId, status: "completed" } }); }
    if (provider === "copilot") {
      event("assistant.usage", { model: "fixture-local", inputTokens: 5, outputTokens: 2 });
      const response = event("assistant.message", { messageId: "answer", content: JSON.stringify(result) });
      const idle = event("session.idle", {});
      reply(runRequest, { response, completionEventId: idle.id, aborted: false });
    }
    if (provider === "custom") reply(runRequest, result);
  };
  const permission = async (kind: "write" | "shell" | "network") => {
    const path = join(process.cwd(), `effect-${kind}.txt`);
    const toolCallId = `tool-${kind}`;
    log("permission", { kind, path });
    let response: any;
    if (provider === "claude") response = await ask("can_use_tool", { tool_name: kind === "write" ? "Write" : kind === "shell" ? "Bash" : "WebFetch", tool_use_id: toolCallId, input: kind === "write" ? { file_path: path, content: "native effect" } : kind === "shell" ? { command: `printf effect > '${path}'` } : { url: "https://fixture.invalid/resource" }, blocked_path: kind === "write" ? path : undefined, decision_reason: "Owned fixture operation" });
    if (provider === "codex") {
      notification("item/started", { threadId: sessionId, item: kind === "write" ? { id: toolCallId, type: "fileChange", changes: [{ path, kind: "add", diff: "+native effect" }] } : { id: toolCallId, type: "commandExecution", command: "owned fixture command", cwd: process.cwd() } });
      response = await ask(kind === "write" ? "item/fileChange/requestApproval" : "item/commandExecution/requestApproval", { itemId: toolCallId, cwd: process.cwd(), command: "owned fixture command", availableDecisions: ["accept", "decline", "cancel"], ...(kind === "network" ? { networkApprovalContext: { host: "fixture.invalid", protocol: "https" } } : {}) });
    }
    if (provider === "copilot") response = await broadcastAsk("permission.requested", { permissionRequest: kind === "write" ? { kind: "write", toolCallId, fileName: path, diff: "+native effect", intention: "Write owned fixture file" } : kind === "shell" ? { kind: "shell", toolCallId, fullCommandText: "owned fixture command", commands: [], possiblePaths: [path], possibleUrls: [], canOfferSessionApproval: false, hasWriteFileRedirection: true, intention: "Run owned fixture command" } : { kind: "url", toolCallId, url: "https://fixture.invalid/resource", intention: "Simulated network permission; no request is made" } });
    if (provider === "custom") response = await ask("permission", { prompt: `Allow fixture ${kind}?`, choices: [{ id: "allow", label: "Allow once", effect: "approve" }, { id: "deny", label: "Deny", effect: "deny" }], tool: { name: toolCallId, kind, paths: [path], diff: "+native effect", urls: kind === "network" ? ["https://fixture.invalid/resource"] : [] }, fields: [] });
    log("permission-response", { kind, response });
    const allowed = provider === "claude" ? response?.behavior === "allow" : provider === "codex" ? response?.decision === "accept" : provider === "copilot" ? response?.kind === "approve-once" : response?.choice === "allow";
    if (allowed && !aborted) { writeFileSync(path, "native effect\n"); log("effect", { kind, path }); }
    return allowed;
  };
  const form = async () => {
    const schema = { type: "object", properties: { count: { type: "integer", minimum: 1, maximum: 5, title: "Count" }, enabled: { type: "boolean", title: "Enabled" }, tags: { type: "array", minItems: 1, maxItems: 2, items: { type: "string", enum: ["a", "b"] } } }, required: ["count", "enabled", "tags"] };
    let response: any;
    if (provider === "claude") response = await ask("elicitation", { mcp_server_name: "fixture", message: "Typed fixture fields", mode: "form", requested_schema: schema });
    if (provider === "codex") response = await ask("mcpServer/elicitation/request", { serverName: "fixture", message: "Typed fixture fields", mode: "form", requestedSchema: schema });
    if (provider === "copilot") response = await broadcastAsk("elicitation.requested", { message: "Typed fixture fields", mode: "form", requestedSchema: schema });
    if (provider === "custom") response = await ask("question", { prompt: "Typed fixture fields", choices: [{ id: "accept", label: "Accept" }], fields: [{ id: "count", label: "Count", type: "integer", minimum: 1, maximum: 5, required: true }, { id: "enabled", label: "Enabled", type: "boolean", required: true }, { id: "tags", label: "Tags", type: "multiselect", options: [{ id: "a", label: "a" }, { id: "b", label: "b" }], minItems: 1, maxItems: 2, required: true }] });
    const content = provider === "custom" ? response.value : response.content;
    assert.deepEqual(content, { count: 3, enabled: false, tags: ["a", "b"] });
    log("form-response", content);
  };
  const question = async () => {
    let response: any;
    if (provider === "claude") { response = await ask("can_use_tool", { tool_name: "AskUserQuestion", tool_use_id: "question-tool", input: { questions: [{ question: "Select fixture labels", header: "Labels", multiSelect: true, options: [{ label: "Alpha", description: "A" }, { label: "Beta", description: "B" }] }] } }); assert.deepEqual(response.updatedInput.answers["Select fixture labels"], ["Alpha", "Beta"]); }
    if (provider === "codex") { response = await ask("item/tool/requestUserInput", { itemId: "question-tool", questions: [{ id: "label", header: "Label", question: "Select fixture label", isOther: false, options: [{ label: "Alpha", description: "A" }, { label: "Beta", description: "B" }] }] }); assert.deepEqual(response.answers.label.answers, ["Beta"]); }
    if (provider === "copilot") { response = await ask("userInput.request", { question: "State fixture answer", requestId: "question-tool", allowFreeform: true }); assert.equal(response.answer, "Answer exactly"); }
    if (provider === "custom") { response = await ask("question", { prompt: "State fixture answer", fields: [{ id: "answer", label: "Answer", type: "text", required: true }] }); assert.equal(response.value?.answer ?? response.answer, "Answer exactly"); }
    log("question-response", response);
  };
  const run = async () => {
    if (!await permission("write")) { finish("blocked"); return; }
    if (!await permission("shell")) { finish("blocked"); return; }
    if (!await permission("network")) { finish("blocked"); return; }
    await form(); await question(); await gotGuidance; finish();
  };
  const begin = (frame: Frame) => { runRequest = frame; void run().catch((error) => { log("scenario-error", String(error)); finish("blocked"); }); };
  const handle = (frame: Frame) => {
    log("received", provider === "copilot" && frame.method === "session.create" ? { method: frame.method, params: { sessionId: frame.params?.sessionId, permissions: frame.params?.permissions, interaction: frame.params?.interaction, events: frame.params?.events, workingDirectory: frame.params?.workingDirectory } } : frame);
    if (provider === "claude") {
      if (frame.type === "control_response") { const response = frame.response; pending.get(response.request_id)?.(response.response); pending.delete(response.request_id); return; }
      if (frame.type === "control_request") {
        const subtype = frame.request.subtype;
        send({ type: "control_response", response: { subtype: "success", request_id: frame.request_id, response: subtype === "initialize" ? { commands: [], models: [], account: {}, output_style: "default", available_output_styles: [] } : {} } });
        if (subtype === "interrupt") { aborted = true; log("native-abort"); send({ type: "result", subtype: "error_during_execution", session_id: sessionId, errors: ["Fixture interrupted"], is_error: true, uuid: crypto.randomUUID(), usage: { input_tokens: 0, output_tokens: 0 } }); }
        return;
      }
      if (frame.type === "user") {
        if (runRequest) { rememberGuidance(String(frame.message.content), frame.uuid); return; }
        sessionId = frame.session_id || sessionId; firstUuid = frame.uuid;
        send({ type: "system", subtype: "init", session_id: sessionId, uuid: crypto.randomUUID(), cwd: process.cwd(), tools: ["Write", "Bash", "WebFetch", "AskUserQuestion"], model: "fixture", permissionMode: "default", mcp_servers: [], apiKeySource: "none", claude_code_version: "2.1.332", slash_commands: [] });
        begin(frame);
      }
      return;
    }
    if (!frame.method) { pending.get(String(frame.id))?.(frame.result); pending.delete(String(frame.id)); return; }
    const params = frame.params ?? {};
    const method = frame.method;
    if (method === "connect" || method === "ping") { reply(frame, { protocolVersion: copilotProtocol, version: copilotProtocol === 4 ? "1.0.116" : "fixture-v3" }); return; }
    if (method === "auth.getStatus") { log("readiness"); reply(frame, { isAuthenticated: true, authType: "fixture" }); return; }
    if (method === "account/read") { assert.equal(params.refreshToken, false); log("readiness"); reply(frame, { account: { type: "fixture" }, requiresOpenaiAuth: true }); return; }
    if (method === "initialize") { reply(frame, provider === "custom" ? { protocol: "karabiner-harness-v2", sessionId, capabilities: { permissions: true, questions: true, steering: "native-turn" } } : { userAgent: "fixture", platformFamily: "unix", platformOs: "macos" }); return; }
    if (method === "initialized") return;
    if (method === "thread/start" || method === "thread/resume") { reply(frame, { thread: { id: sessionId, sessionId } }); return; }
    if (method === "session.create" || method === "session.resume") {
      // @microsoft/copilot-sdk 2.0.3's generated v4 request groups session options.
      // Reject a v3-shaped peer request rather than merely changing the handshake number.
      assert.equal(params.permissions?.request, true);
      assert.equal(params.interaction?.requestUserInput, true);
      assert.equal(params.interaction?.requestElicitation, true);
      assert.equal(params.events?.streaming, true);
      sessionId = params.sessionId; reply(frame, { sessionId, workspacePath: process.cwd(), capabilities: { ui: { elicitation: true } } }); return;
    }
    if (method === "session.sendAndWait" || method === "turn/start" || method === "run") {
      if (provider === "codex") { reply(frame, { turn: { id: turnId, status: "inProgress" } }); notification("turn/started", { threadId: sessionId, turn: { id: turnId, status: "inProgress" } }); }
      begin(frame); return;
    }
    if (method === "session.send" || method === "turn/steer" || method === "instruction") { rememberGuidance(params.prompt ?? params.input?.[0]?.text ?? params.message, params.clientTurnId ?? params.id); reply(frame, { messageId: "steering", turnId, delivered: true }); return; }
    if (method === "session.permissions.handlePendingPermissionRequest" || method === "session.ui.handlePendingElicitation") { pending.get(params.requestId)?.(params.result); pending.delete(params.requestId); reply(frame, {}); return; }
    if (method === "session.abort" || method === "turn/interrupt" || method === "abort") {
      aborted = true; log("native-abort"); reply(frame, {});
      if (provider === "codex") notification("turn/completed", { threadId: sessionId, turn: { id: turnId, status: "interrupted" } });
      if (provider === "copilot" && runRequest) reply(runRequest, { aborted: true });
      if (provider === "custom" && runRequest) reply(runRequest, outcome("blocked"));
      return;
    }
    if (method === "session.detach") { reply(frame, process.argv.includes("--fixture-cleanup-failure") ? { success: false, error: "fixture-detach-failure-after-completion" } : { success: true }); return; }
    if (method === "session.destroy") { reply(frame, { success: true }); return; }
    if (method === "runtime.shutdown") { log("runtime-shutdown"); reply(frame, null); return; }
    if (frame.id !== undefined) send({ jsonrpc: "2.0", id: frame.id, error: { code: -32601, message: `Fixture does not implement ${method}` } });
  };
  let buffer = Buffer.alloc(0);
  process.stdin.on("data", (chunk: Buffer) => {
    buffer = Buffer.concat([buffer, chunk]);
    while (buffer.length) {
      if (provider === "copilot") {
        const split = buffer.indexOf("\r\n\r\n"); if (split < 0) break;
        const length = Number(/Content-Length:\s*(\d+)/i.exec(buffer.subarray(0, split).toString())?.[1]);
        if (!Number.isFinite(length) || length > 4 * 1024 * 1024) throw new Error("Invalid fixture frame length");
        if (buffer.length < split + 4 + length) break;
        const value = buffer.subarray(split + 4, split + 4 + length).toString(); buffer = buffer.subarray(split + 4 + length); handle(JSON.parse(value));
      } else {
        const split = buffer.indexOf("\n"); if (split < 0) break;
        const line = buffer.subarray(0, split).toString(); buffer = buffer.subarray(split + 1); if (line.trim()) handle(JSON.parse(line));
      }
    }
  });
  process.stdin.on("end", () => { log("eof"); process.exit(0); });
  process.on("SIGTERM", () => { log("terminated"); process.exit(0); });
}
