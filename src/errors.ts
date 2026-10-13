import { ErrorReport } from "./api";

export function normalizeError(error: unknown): ErrorReport {
  const value = error && typeof error === "object" ? error as Partial<ErrorReport> : {};
  return {
    code: typeof value.code === "string" ? value.code : "UnexpectedError",
    message: typeof value.message === "string" && value.message ? value.message :
      typeof error === "string" && error ? error : "An unexpected error occurred.",
    detail: typeof value.detail === "string" ? value.detail : "",
    retryable: typeof value.retryable === "boolean" ? value.retryable : false,
  };
}
