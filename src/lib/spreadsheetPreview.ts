export interface SpreadsheetPreviewSheet {
  name: string;
  rows: string[][];
}

export interface SpreadsheetPreviewWorkbook {
  SheetNames: string[];
  Sheets: Record<string, unknown>;
}

export interface SpreadsheetPreviewUtils {
  sheet_to_json: (
    sheet: unknown,
    options: {
      header: 1;
      raw: false;
      defval: string;
      blankrows: false;
    },
  ) => unknown[][];
}

function normalizeCell(value: unknown): string {
  return value == null ? "" : String(value);
}

function normalizeRows(rows: unknown[][]): string[][] {
  return rows.map((row) => (Array.isArray(row) ? row.map(normalizeCell) : [normalizeCell(row)]));
}

export function workbookToPreviewSheets(
  workbook: SpreadsheetPreviewWorkbook,
  utils: SpreadsheetPreviewUtils,
): SpreadsheetPreviewSheet[] {
  return workbook.SheetNames.map((name) => {
    const sheet = workbook.Sheets[name];
    const rows = sheet
      ? utils.sheet_to_json(sheet, {
          header: 1,
          raw: false,
          defval: "",
          blankrows: false,
        })
      : [];
    return {
      name,
      rows: normalizeRows(rows),
    };
  });
}
