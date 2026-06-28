type SearchableDocument = {
  filename: string;
  source_path: string;
  display_name?: string | null;
  category?: string | null;
};

type SearchableMark = {
  category?: string | null;
  parties?: string[];
  evidenceAttitude?: string | null;
  submissionStage?: string | null;
};

function normalizeSearchText(value: string): string {
  return value.trim().toLowerCase();
}

export function documentMatchesSearch(
  doc: SearchableDocument,
  mark: SearchableMark,
  query: string,
): boolean {
  const q = normalizeSearchText(query);
  if (!q) return true;
  const hay = [
    doc.display_name,
    doc.filename,
    doc.source_path,
    doc.category,
    mark.category,
    ...(mark.parties ?? []),
    mark.evidenceAttitude,
    mark.submissionStage,
  ]
    .filter((v): v is string => typeof v === "string" && v.trim().length > 0)
    .join(" ")
    .toLowerCase();
  return hay.includes(q);
}
