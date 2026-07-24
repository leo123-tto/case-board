import type { WordTemplate } from "@/lib/api";

export interface AiWorkspace {
  id: string;
  title: string;
  description: string | null;
  is_favorite: number;
  last_opened_at: string | null;
  last_document_id: string | null;
  last_conversation_id: string | null;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
}

export interface AiWorkspaceSummary extends AiWorkspace {
  source_count: number;
  artifact_count: number;
  conversation_count: number;
}

export type WorkspaceListViewMode = "all" | "recent";

export interface ListAiWorkspacesInput {
  query: string | null;
  view: WorkspaceListViewMode;
  include_archived: boolean;
}

export interface CreateAiWorkspaceInput {
  title: string;
  description: string | null;
}

export interface UpdateAiWorkspaceInput {
  title?: string;
  description?: string;
  is_favorite?: boolean;
}

export type AiWorkspaceDocumentKind = "source" | "artifact";
export type AiWorkspaceExtractionStatus =
  | "queued"
  | "processing"
  | "ready"
  | "review"
  | "failed"
  | "missing"
  | "not_required";

export interface AiWorkspaceDocument {
  id: string;
  workspace_id: string;
  kind: AiWorkspaceDocumentKind;
  title: string;
  filename: string;
  mime_type: string | null;
  size_bytes: number | null;
  source_path: string | null;
  normalized_source_path: string | null;
  content_path: string | null;
  extracted_text_path: string | null;
  extraction_status: AiWorkspaceExtractionStatus;
  last_error: string | null;
  missing: number;
  quality_status: string | null;
  working_copy_revision: number;
  working_copy_hash: string | null;
  latest_version_no: number;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
}

export interface SourceAddError {
  path: string;
  error: string;
}

export interface AddAiWorkspaceSourcesResult {
  added: AiWorkspaceDocument[];
  errors: SourceAddError[];
  preferred_export_dir: string | null;
}

export interface AiWorkspaceExportPaths {
  preferred_export_dir: string | null;
  docx_path: string | null;
  docx_word_template: WordTemplate | null;
  html_path: string | null;
}

export interface AiWorkspaceExportWrite {
  format: "docx" | "html";
  path: string;
}

export interface AiWorkspaceExportWriteError extends AiWorkspaceExportWrite {
  error: string;
}

export interface AiWorkspaceExportRefreshResult {
  written: AiWorkspaceExportWrite[];
  errors: AiWorkspaceExportWriteError[];
}

export interface AiWorkspaceDocumentProgress {
  workspace_id: string;
  document_id: string;
  filename: string;
  status: AiWorkspaceExtractionStatus;
  error: string | null;
}

export interface AiWorkspaceArtifactContent {
  document: AiWorkspaceDocument;
  markdown: string;
}

export interface CreateAiWorkspaceArtifactInput {
  title: string;
  initial_markdown: string | null;
}

export interface SaveAiWorkspaceArtifactInput {
  title: string;
  markdown: string;
  expected_revision: number;
}

export interface CreateAiWorkspaceArtifactVersionInput {
  trigger: "manual" | "leave" | "export" | "ai" | "before_ai" | "after_ai" | "restore";
  summary: string | null;
}

export interface AiWorkspaceDocumentVersion {
  id: string;
  document_id: string;
  version_no: number;
  content_md: string;
  created_by: string;
  trigger: string;
  change_summary: string;
  source_snapshot_json: string;
  message_id: string | null;
  created_at: string;
}

export interface WorkspaceReference {
  document_id: string;
  page_no: number | null;
}

export interface RetrievedWorkspaceSource {
  document_id: string;
  title: string;
  page_no: number | null;
  excerpt: string;
  content_hash: string;
  score: number;
  selection: "manual" | "auto";
  source_missing: boolean;
}

export interface AiWorkspaceConversation {
  id: string;
  workspace_id: string;
  title: string;
  title_is_manual: number;
  last_message_at: string | null;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
}

export type AiWorkspaceMessageStatus =
  | "queued"
  | "streaming"
  | "completed"
  | "incomplete"
  | "failed"
  | "cancelled";

export interface WorkspaceCitation {
  ref: number;
  type: "law" | "case" | "doc" | "kb_local" | "web" | string;
  source: string;
  quote: string | null;
  court: string | null;
  url: string | null;
  verified: boolean;
  tool_call_id?: string | null;
}

export interface WorkspaceToolCallRecord {
  tool: string;
  args: unknown;
  kb_hit: boolean;
  credits_used: number;
  success: boolean;
  error_short: string | null;
  result_preview?: unknown;
  started_at_ms: number;
  finished_at_ms: number;
}

export interface WorkspaceAskQuestion {
  question: string;
  options: string[];
  allow_input: boolean;
  multiple: boolean;
  min_selections?: number | null;
  max_selections?: number | null;
}

export interface AiWorkspaceMessage {
  id: string;
  conversation_id: string;
  role: "user" | "assistant" | "system";
  content: string;
  status: AiWorkspaceMessageStatus;
  attached_document_ids_json: string;
  citations_json: string;
  artifact_document_id: string | null;
  model: string | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  latency_ms: number | null;
  error_short: string | null;
  task_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface AiWorkspaceTask {
  id: string;
  workspace_id: string;
  conversation_id: string;
  assistant_message_id: string;
  status: AiWorkspaceMessageStatus;
  input_json: string;
  tool_calls_json: string;
  error_short: string | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
}

export interface AiWorkspaceChatInput {
  workspace_id: string;
  conversation_id: string;
  user_message: string;
  user_message_id: string;
  message_id: string;
  references: WorkspaceReference[];
  editing_document_id: string | null;
  skill_name?: string | null;
}

export interface AiWorkspaceChatResult {
  user_message_id: string;
  assistant_message_id: string;
  task_id: string;
  model: string;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  latency_ms: number;
  citations: WorkspaceCitation[];
  sources: RetrievedWorkspaceSource[];
  tool_calls: WorkspaceToolCallRecord[];
  ask_user: WorkspaceAskQuestion[] | null;
  artifact_doc_id: string | null;
}

export interface AiWorkspaceDocumentProposal {
  id: string;
  workspace_id: string;
  document_id: string;
  conversation_id: string | null;
  message_id: string | null;
  base_revision: number;
  base_content_hash: string;
  proposed_markdown: string;
  summary: string;
  source_snapshot_json: string;
  status: "pending" | "accepted" | "rejected" | "superseded";
  resolved_markdown: string | null;
  created_at: string;
  resolved_at: string | null;
}

export type AiWorkspaceChatStreamEvent =
  | { kind: "activity"; activity: import("@/lib/api").ChatActivity }
  | { kind: "delta"; text: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool_call"; record: WorkspaceToolCallRecord }
  | { kind: "ask_user"; questions: WorkspaceAskQuestion[] }
  | {
      kind: "done";
      prompt_tokens: number | null;
      completion_tokens: number | null;
      model: string;
    }
  | { kind: "error"; message: string };
