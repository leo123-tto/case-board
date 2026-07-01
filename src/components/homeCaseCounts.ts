import type { StatusId } from "@/modules/litigation/lib/inferStatus";

interface CaseStatusLike {
  status: {
    id: StatusId;
  };
}

export function isOpenCaseStatus(statusId: StatusId): boolean {
  return statusId !== "closed";
}

export function countOpenCaseRows<T extends CaseStatusLike>(rows: T[]): number {
  return rows.filter((row) => isOpenCaseStatus(row.status.id)).length;
}
