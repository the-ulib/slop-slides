export const PERMISSION_MODES = {
  ask: { label: "Ask for approval", description: "Work in the deck workspace; ask for extra access." },
  autoReview: { label: "Approve for me", description: "Codex reviews eligible requests for extra access." },
  fullAccess: { label: "Full access", description: "Run without sandbox restrictions or approval prompts." },
  custom: { label: "Custom", description: "Use your existing Codex configuration." },
} as const;
export type PermissionMode = keyof typeof PERMISSION_MODES;
export type ApprovalDecision = "accept" | "acceptForSession" | "decline";
export interface Approval {
  id: string;
  title: string;
  reason: string | null;
  details: string;
  acceptLabel: string;
  decisions: ApprovalDecision[];
}
export function isPermissionMode(value: unknown): value is PermissionMode {
  return typeof value === "string" && Object.hasOwn(PERMISSION_MODES, value);
}
