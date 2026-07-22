import { invoke } from "@tauri-apps/api/core";

import type {
  AddAiWorkspaceSourcesResult,
  AiWorkspaceChatInput,
  AiWorkspaceChatResult,
  AiWorkspaceArtifactContent,
  AiWorkspaceConversation,
  AiWorkspace,
  AiWorkspaceDocument,
  AiWorkspaceDocumentProposal,
  AiWorkspaceDocumentVersion,
  AiWorkspaceMessage,
  AiWorkspaceTask,
  CreateAiWorkspaceArtifactInput,
  CreateAiWorkspaceArtifactVersionInput,
  AiWorkspaceSummary,
  CreateAiWorkspaceInput,
  ListAiWorkspacesInput,
  SaveAiWorkspaceArtifactInput,
  UpdateAiWorkspaceInput,
} from "./types";

export function listAiWorkspaces(
  input: ListAiWorkspacesInput,
): Promise<AiWorkspaceSummary[]> {
  return invoke<AiWorkspaceSummary[]>("list_ai_workspaces", { input });
}

export function createAiWorkspace(
  input: CreateAiWorkspaceInput,
): Promise<AiWorkspace> {
  return invoke<AiWorkspace>("create_ai_workspace", { input });
}

export function openAiWorkspace(workspaceId: string): Promise<AiWorkspace> {
  return invoke<AiWorkspace>("open_ai_workspace", { workspaceId });
}

export function updateAiWorkspace(
  workspaceId: string,
  input: UpdateAiWorkspaceInput,
): Promise<AiWorkspace> {
  return invoke<AiWorkspace>("update_ai_workspace", { workspaceId, input });
}

export function archiveAiWorkspace(workspaceId: string): Promise<void> {
  return invoke<void>("archive_ai_workspace", { workspaceId });
}

export function listAiWorkspaceConversations(
  workspaceId: string,
): Promise<AiWorkspaceConversation[]> {
  return invoke<AiWorkspaceConversation[]>("list_ai_workspace_conversations", {
    workspaceId,
  });
}

export function ensureAiWorkspaceConversation(
  workspaceId: string,
): Promise<AiWorkspaceConversation> {
  return invoke<AiWorkspaceConversation>("ensure_ai_workspace_conversation", {
    workspaceId,
  });
}

export function createAiWorkspaceConversation(
  workspaceId: string,
  title: string | null = null,
): Promise<AiWorkspaceConversation> {
  return invoke<AiWorkspaceConversation>("create_ai_workspace_conversation", {
    workspaceId,
    title,
  });
}

export function renameAiWorkspaceConversation(
  workspaceId: string,
  conversationId: string,
  title: string,
): Promise<AiWorkspaceConversation> {
  return invoke<AiWorkspaceConversation>("rename_ai_workspace_conversation", {
    workspaceId,
    conversationId,
    title,
  });
}

export function selectAiWorkspaceConversation(
  workspaceId: string,
  conversationId: string,
): Promise<void> {
  return invoke<void>("select_ai_workspace_conversation", {
    workspaceId,
    conversationId,
  });
}

export function archiveAiWorkspaceConversation(
  workspaceId: string,
  conversationId: string,
): Promise<void> {
  return invoke<void>("archive_ai_workspace_conversation", {
    workspaceId,
    conversationId,
  });
}

export function listAiWorkspaceMessages(
  workspaceId: string,
  conversationId: string,
): Promise<AiWorkspaceMessage[]> {
  return invoke<AiWorkspaceMessage[]>("list_ai_workspace_messages", {
    workspaceId,
    conversationId,
  });
}

export function listAiWorkspaceTasks(
  workspaceId: string,
  conversationId: string,
): Promise<AiWorkspaceTask[]> {
  return invoke<AiWorkspaceTask[]>("list_ai_workspace_tasks", {
    workspaceId,
    conversationId,
  });
}

export function runAiWorkspaceChat(
  input: AiWorkspaceChatInput,
): Promise<AiWorkspaceChatResult> {
  return invoke<AiWorkspaceChatResult>("ai_workspace_chat", { input });
}

export function cancelAiWorkspaceChat(messageId: string): Promise<boolean> {
  return invoke<boolean>("cancel_ai_workspace_chat", { messageId });
}

export function steerAiWorkspaceChat(input: {
  messageId: string;
  workspaceId: string;
  conversationId: string;
  content: string;
}): Promise<string> {
  return invoke<string>("steer_ai_workspace_chat", input);
}

export function addAiWorkspaceSources(
  workspaceId: string,
  paths: string[],
): Promise<AddAiWorkspaceSourcesResult> {
  return invoke<AddAiWorkspaceSourcesResult>("add_ai_workspace_sources", {
    workspaceId,
    paths,
  });
}

export function listAiWorkspaceDocuments(
  workspaceId: string,
): Promise<AiWorkspaceDocument[]> {
  return invoke<AiWorkspaceDocument[]>("list_ai_workspace_documents", {
    workspaceId,
  });
}

export function retryAiWorkspaceSource(
  workspaceId: string,
  documentId: string,
): Promise<void> {
  return invoke<void>("retry_ai_workspace_source", { workspaceId, documentId });
}

export function relinkAiWorkspaceSource(
  workspaceId: string,
  documentId: string,
  path: string,
): Promise<AiWorkspaceDocument> {
  return invoke<AiWorkspaceDocument>("relink_ai_workspace_source", {
    workspaceId,
    documentId,
    path,
  });
}

export function archiveAiWorkspaceDocument(
  workspaceId: string,
  documentId: string,
): Promise<void> {
  return invoke<void>("archive_ai_workspace_document", {
    workspaceId,
    documentId,
  });
}

export function readAiWorkspaceText(
  workspaceId: string,
  documentId: string,
): Promise<string> {
  return invoke<string>("read_ai_workspace_text", { workspaceId, documentId });
}

export function allowAiWorkspaceAssets(
  workspaceId: string,
  documentId: string,
): Promise<void> {
  return invoke<void>("allow_ai_workspace_assets", { workspaceId, documentId });
}

export function readAiWorkspaceFileBytes(
  workspaceId: string,
  documentId: string,
): Promise<number[]> {
  return invoke<number[]>("read_ai_workspace_file_bytes", { workspaceId, documentId });
}

export function createAiWorkspaceArtifact(
  workspaceId: string,
  input: CreateAiWorkspaceArtifactInput,
): Promise<AiWorkspaceArtifactContent> {
  return invoke<AiWorkspaceArtifactContent>("create_ai_workspace_artifact", {
    workspaceId,
    input,
  });
}

export function createAiWorkspaceArtifactFromMessage(
  workspaceId: string,
  messageId: string,
  title: string,
): Promise<AiWorkspaceArtifactContent> {
  return invoke<AiWorkspaceArtifactContent>(
    "create_ai_workspace_artifact_from_message",
    { workspaceId, messageId, title },
  );
}

export function readAiWorkspaceArtifact(
  workspaceId: string,
  documentId: string,
): Promise<AiWorkspaceArtifactContent> {
  return invoke<AiWorkspaceArtifactContent>("read_ai_workspace_artifact", {
    workspaceId,
    documentId,
  });
}

export function saveAiWorkspaceArtifact(
  workspaceId: string,
  documentId: string,
  input: SaveAiWorkspaceArtifactInput,
): Promise<AiWorkspaceArtifactContent> {
  return invoke<AiWorkspaceArtifactContent>("save_ai_workspace_artifact", {
    workspaceId,
    documentId,
    input,
  });
}

export function createAiWorkspaceArtifactVersion(
  workspaceId: string,
  documentId: string,
  input: CreateAiWorkspaceArtifactVersionInput,
): Promise<AiWorkspaceDocumentVersion> {
  return invoke<AiWorkspaceDocumentVersion>("create_ai_workspace_artifact_version", {
    workspaceId,
    documentId,
    input,
  });
}

export function listAiWorkspaceArtifactVersions(
  workspaceId: string,
  documentId: string,
): Promise<AiWorkspaceDocumentVersion[]> {
  return invoke<AiWorkspaceDocumentVersion[]>("list_ai_workspace_artifact_versions", {
    workspaceId,
    documentId,
  });
}

export function restoreAiWorkspaceArtifactVersion(
  workspaceId: string,
  documentId: string,
  versionId: string,
  expectedRevision: number,
): Promise<AiWorkspaceArtifactContent> {
  return invoke<AiWorkspaceArtifactContent>("restore_ai_workspace_artifact_version", {
    workspaceId,
    documentId,
    versionId,
    expectedRevision,
  });
}

export function createAiWorkspaceDocumentProposal(
  workspaceId: string,
  documentId: string,
  conversationId: string,
  messageId: string,
): Promise<AiWorkspaceDocumentProposal> {
  return invoke<AiWorkspaceDocumentProposal>("create_ai_workspace_document_proposal", {
    workspaceId,
    documentId,
    conversationId,
    messageId,
  });
}

export function listAiWorkspaceDocumentProposals(
  workspaceId: string,
  documentId: string,
): Promise<AiWorkspaceDocumentProposal[]> {
  return invoke<AiWorkspaceDocumentProposal[]>("list_ai_workspace_document_proposals", {
    workspaceId,
    documentId,
  });
}

export function resolveAiWorkspaceDocumentProposal(
  workspaceId: string,
  proposalId: string,
  action: "accepted" | "rejected",
  resolvedMarkdown: string | null,
): Promise<AiWorkspaceArtifactContent | null> {
  return invoke<AiWorkspaceArtifactContent | null>("resolve_ai_workspace_document_proposal", {
    workspaceId,
    proposalId,
    action,
    resolvedMarkdown,
  });
}
