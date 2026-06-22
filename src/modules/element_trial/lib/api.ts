/**
 * 要素式审判智能辅助 — API 层
 * 封装与 Rust 后端的 invoke 调用
 */

import { invoke } from "@tauri-apps/api/core";

// ───── 类型定义 ─────

export interface ElementTemplate {
  id: string;
  cause: string;
  direction: string;
  element_name: string;
  element_desc: string;
  is_required: boolean;
  evidence_type: string | null;
  evidence_hint: string | null;
  burden_party: string | null;
  sort_order: number;
}

export interface ElementFact {
  id: string;
  case_id: string;
  stage: string | null;
  template_id: string | null;
  fact_name: string;
  fact_desc: string | null;
  claim_party: string | null;
  evidence_ids: string | null;
  proof_status: string;
  opponent_rebuttal: string | null;
  court_finding: string | null;
  is_established: boolean | null;
  is_disputed: boolean;
  notes: string | null;
}

export interface TrialStrategy {
  id: string;
  case_id: string;
  stage: string | null;
  strategy_layer: string;
  strategy_content: string;
  target_fact_ids: string | null;
  predicted_opponent_strategy: string | null;
  evidence_gap_analysis: string | null;
  recommended_actions: string | null;
  risk_level: string | null;
  is_adopted: boolean;
}

export interface ElementComplaint {
  id: string;
  case_id: string;
  doc_type: string;
  direction: string;
  content_md: string;
  filled_elements: string | null;
  version: number;
  is_final: boolean;
}

// ───── API 调用 ─────

export async function getElementTemplates(
  cause: string,
  direction?: string,
): Promise<ElementTemplate[]> {
  return invoke("get_element_templates", { cause, direction: direction ?? null });
}

export async function listTemplateCauses(): Promise<string[]> {
  return invoke("list_template_causes");
}

export async function getElementFacts(caseId: string): Promise<ElementFact[]> {
  return invoke("get_element_facts", { caseId });
}

export async function getDisputedFacts(caseId: string): Promise<ElementFact[]> {
  return invoke("get_disputed_facts", { caseId });
}

export async function getTrialStrategies(caseId: string): Promise<TrialStrategy[]> {
  return invoke("get_trial_strategies", { caseId });
}

export async function upsertElementFacts(facts: ElementFact[]): Promise<void> {
  return invoke("upsert_element_facts", { facts });
}

export async function saveTrialStrategy(strategy: TrialStrategy): Promise<void> {
  return invoke("save_trial_strategy", { strategy });
}

export async function getElementComplaints(caseId: string): Promise<ElementComplaint[]> {
  return invoke("get_element_complaints", { caseId });
}
