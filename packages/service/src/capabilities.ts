import { API_VERSION, type ExecutionTargetInfo, type HarnessInfo, type ServiceMeta } from "@karabiner/contracts";
import manifest from "../package.json";

export const metadata: ServiceMeta = {
  apiVersion: API_VERSION, serviceVersion: manifest.version,
  capabilities: { storage: true, configuration: true, documents: true, codeTours: true, interventionIntents: true, activity: true, statistics: true, events: true,
    scheduler: false, harnessExecution: false, integrations: false, virtualMachines: false },
};
export const harnesses: HarnessInfo[] = [
  { id: "claude", name: "Claude Code", executable: "claude" },
  { id: "codex", name: "Codex", executable: "codex" },
  { id: "copilot", name: "GitHub Copilot", executable: "copilot" },
  { id: "custom", name: "Custom local harness", executable: "" },
].map(item => ({ ...item, installed: null, authenticated: null, status: "not_checked", executionAvailable: false,
  message: "Local installation and authentication have not been checked. Connect a harness adapter to enable execution." } as HarnessInfo));
export const executionTargets: ExecutionTargetInfo[] = [
  { id: "host", name: "This Mac", kind: "host", available: false, requirements: "An installed, authenticated harness", message: "Host executor is not implemented." },
  { id: "mxc", name: "MXC Seatbelt", kind: "process_sandbox", available: false, requirements: "macOS 15+; MXC runtime", message: "Process sandbox adapter is not implemented. This is not a VM." },
  { id: "apple-container", name: "Apple Container", kind: "virtual_machine", available: false, requirements: "Apple silicon; macOS 26+; container CLI/service", message: "Linux VM adapter is not implemented." },
];
