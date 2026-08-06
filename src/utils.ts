/**
 * Mirrors the backend's profile-naming collapse rule (SPEC.md §5) so the UI
 * can preview/select a profile name (e.g. for the "Default profile" picker)
 * before `apply_profiles` runs. The backend remains the source of truth for
 * the names actually written to ~/.aws/config.
 */
export function computeProfileName(
  accountName: string,
  roleName: string,
): string {
  const accountParts = accountName.split("-");
  const roleParts = roleName.split("-");
  const maxOverlap = Math.min(accountParts.length, roleParts.length);

  let overlap = 0;
  for (let n = maxOverlap; n > 0; n--) {
    const suffix = accountParts
      .slice(accountParts.length - n)
      .join("-")
      .toLowerCase();
    const prefix = roleParts.slice(0, n).join("-").toLowerCase();
    if (suffix === prefix) {
      overlap = n;
      break;
    }
  }

  const remainder = roleParts.slice(overlap);
  return remainder.length > 0
    ? `${accountName}-${remainder.join("-")}`
    : accountName;
}

export function selectionKey(accountId: string, roleName: string): string {
  return `${accountId}::${roleName}`;
}

/** Formats a seconds count as e.g. "5h 12m 03s" / "12m 03s" / "42s". */
export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(s / 3600);
  const minutes = Math.floor((s % 3600) / 60);
  const seconds = s % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  if (hours > 0) return `${hours}h ${pad(minutes)}m ${pad(seconds)}s`;
  if (minutes > 0) return `${minutes}m ${pad(seconds)}s`;
  return `${seconds}s`;
}

/** Detects the export_credentials format to request based on the platform. */
export function detectExportFormat(): "env" | "powershell" {
  return navigator.userAgent.includes("Windows") ? "powershell" : "env";
}
