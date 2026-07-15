import type { Settings } from "./types";

export type OcrKeyIssue = {
  label: string;
  reason: "missing" | "unverified";
};

type OcrSettingsForImport = Pick<
  Settings,
  | "ocr_cloud_primary"
  | "mineru_api_key"
  | "mineru_verified_at"
  | "paddle_vl_api_key"
  | "paddle_vl_verified_at"
>;

/** 导入前只校验当前主力 OCR；备用线路始终选填。 */
export function primaryOcrIssues(settings: OcrSettingsForImport): OcrKeyIssue[] {
  const paddlePrimary = settings.ocr_cloud_primary === "paddle-vl";
  const label = paddlePrimary
    ? "PaddleOCR 访问令牌(云端 OCR)"
    : "MinerU API Token(云端 OCR)";
  const filled = paddlePrimary
    ? Boolean(settings.paddle_vl_api_key?.trim())
    : Boolean(settings.mineru_api_key?.trim());
  const verified = paddlePrimary
    ? Boolean(settings.paddle_vl_verified_at)
    : Boolean(settings.mineru_verified_at);

  if (!filled) return [{ label, reason: "missing" }];
  if (!verified) return [{ label, reason: "unverified" }];
  return [];
}
