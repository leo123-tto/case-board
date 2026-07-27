import { useEffect, useState } from "react";

import { credentialStatusLabel } from "@/lib/credentials";
import type { CredentialStatusView } from "@/lib/types";

interface CredentialFieldProps {
  label: string;
  status: CredentialStatusView | null;
  onSave: (secretInput: string) => Promise<unknown>;
  onVerify?: (handle: string, revision: number) => Promise<unknown>;
  onRevoke: (handle: string, revision: number) => Promise<unknown>;
  placeholder?: string;
  hint?: string;
  disabled?: boolean;
  multiline?: boolean;
}

export function CredentialField({
  label,
  status,
  onSave,
  onVerify,
  onRevoke,
  placeholder = "输入新凭据；留空保持现有凭据不变",
  hint,
  disabled = false,
  multiline = false,
}: CredentialFieldProps) {
  const [secretInput, setSecretInput] = useState("");
  const [busy, setBusy] = useState<"save" | "verify" | "revoke" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setSecretInput("");
  }, [status?.handle, status?.revision]);

  const save = async () => {
    const next = secretInput.trim();
    if (!next) return;
    setBusy("save");
    setError(null);
    try {
      await onSave(next);
      setSecretInput("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const revoke = async () => {
    if (!status) return;
    setBusy("revoke");
    setError(null);
    try {
      await onRevoke(status.handle, status.revision);
      setSecretInput("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const verify = async () => {
    if (!status || !onVerify) return;
    setBusy("verify");
    setError(null);
    try {
      await onVerify(status.handle, status.revision);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={`credential-${label}`} className="text-sm font-medium text-foreground">
          {label}
        </label>
        <span className="text-xs text-muted-foreground">{credentialStatusLabel(status)}</span>
      </div>
      <div className="flex gap-2">
        {multiline ? (
          <textarea
            id={`credential-${label}`}
            rows={3}
            value={secretInput}
            onChange={(event) => setSecretInput(event.target.value)}
            placeholder={placeholder}
            autoComplete="off"
            spellCheck={false}
            disabled={disabled || busy !== null}
            className="min-w-0 flex-1 rounded-md border border-border bg-background px-3 py-2 font-mono text-xs outline-none focus:border-foreground/40 disabled:opacity-50"
          />
        ) : (
          <input
            id={`credential-${label}`}
            type="password"
            value={secretInput}
            onChange={(event) => setSecretInput(event.target.value)}
            placeholder={placeholder}
            autoComplete="new-password"
            disabled={disabled || busy !== null}
            className="min-w-0 flex-1 rounded-md border border-border bg-background px-3 py-2 text-sm outline-none focus:border-foreground/40 disabled:opacity-50"
          />
        )}
        <button
          type="button"
          onClick={() => void save()}
          disabled={disabled || busy !== null || !secretInput.trim()}
          aria-label={`保存${label}`}
          className="rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background disabled:opacity-50"
        >
          {busy === "save" ? "保存中" : "保存"}
        </button>
        {status && (
          onVerify && (
            <button
              type="button"
              onClick={() => void verify()}
              disabled={disabled || busy !== null}
              aria-label={`验证${label}`}
              className="rounded-md border border-border px-3 py-2 text-sm disabled:opacity-50"
            >
              {busy === "verify" ? "验证中" : "验证"}
            </button>
          )
        )}
        {status && (
          <button
            type="button"
            onClick={() => void revoke()}
            disabled={disabled || busy !== null}
            aria-label={`清除${label}`}
            className="rounded-md border border-border px-3 py-2 text-sm text-destructive disabled:opacity-50"
          >
            {busy === "revoke" ? "清除中" : "清除"}
          </button>
        )}
      </div>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    </div>
  );
}
