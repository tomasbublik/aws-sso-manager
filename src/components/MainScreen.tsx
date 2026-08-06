import { useEffect, useState } from "react";
import {
  applyProfiles,
  copyText,
  exportCredentials,
  getManagedProfiles,
  listAccounts,
  refreshTokenNow,
  ssoLogout,
  verifyProfile,
  type AccountInfo,
  type AppState,
  type ProfileInfo,
  type Selection,
  type TokenStatus,
} from "../api";
import { branding } from "../branding";
import { Banner } from "./Banner";
import { Spinner } from "./Spinner";
import {
  computeProfileName,
  detectExportFormat,
  formatDuration,
  selectionKey,
} from "../utils";

interface MainScreenProps {
  sessionName: string;
  cliInstalled: boolean;
  token: TokenStatus;
  onTokenUpdate: (status: TokenStatus) => void;
  onLogout: (state: AppState) => void;
  onError: (message: string) => void;
}

const REGION_PRESETS: readonly string[] = branding.defaults.regionPresets;
const CUSTOM_REGION = "__custom__";
const NO_DEFAULT = "";

interface AppliedProfile extends ProfileInfo {
  status: "pending" | "ok" | "error";
  resolvedArn?: string;
  errorMessage?: string;
}

export function MainScreen({
  sessionName,
  cliInstalled,
  token,
  onTokenUpdate,
  onLogout,
  onError,
}: MainScreenProps) {
  const [accounts, setAccounts] = useState<AccountInfo[] | null>(null);
  const [selections, setSelections] = useState<Map<string, Selection>>(new Map());
  const [regionChoice, setRegionChoice] = useState<string>(
    branding.defaults.ssoRegion || REGION_PRESETS[0] || "eu-central-1",
  );
  const [customRegion, setCustomRegion] = useState("");
  const [defaultKey, setDefaultKey] = useState<string>(NO_DEFAULT);
  const [applying, setApplying] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [appliedProfiles, setAppliedProfiles] = useState<AppliedProfile[]>([]);
  const [copiedFlash, setCopiedFlash] = useState<string | null>(null);
  const [displaySeconds, setDisplaySeconds] = useState(token.remainingSeconds);

  // Live 1s ticker, reset whenever a fresh remainingSeconds arrives.
  useEffect(() => {
    setDisplaySeconds(token.remainingSeconds);
    const interval = setInterval(() => {
      setDisplaySeconds((s) => (s > 0 ? s - 1 : 0));
    }, 1000);
    return () => clearInterval(interval);
  }, [token.remainingSeconds, token.expiresAt]);

  // Initial data load.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [accountList, managedProfiles] = await Promise.all([
          listAccounts(),
          getManagedProfiles(),
        ]);
        if (cancelled) return;
        setAccounts(accountList);

        const preChecked = new Map<string, Selection>();
        for (const account of accountList) {
          for (const role of account.roles) {
            const match = managedProfiles.find(
              (p) => p.accountId === account.accountId && p.roleName === role,
            );
            if (match) {
              preChecked.set(selectionKey(account.accountId, role), {
                accountId: account.accountId,
                accountName: account.accountName,
                roleName: role,
              });
              if (match.isDefault) {
                setDefaultKey(selectionKey(account.accountId, role));
              }
              if (match.region) {
                setRegionChoice(
                  REGION_PRESETS.includes(match.region) ? match.region : CUSTOM_REGION,
                );
                if (!REGION_PRESETS.includes(match.region)) {
                  setCustomRegion(match.region);
                }
              }
            }
          }
        }
        setSelections(preChecked);
      } catch (err) {
        if (!cancelled) onError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Drop a stale default selection if the role got unchecked.
  useEffect(() => {
    if (defaultKey !== NO_DEFAULT && !selections.has(defaultKey)) {
      setDefaultKey(NO_DEFAULT);
    }
  }, [selections, defaultKey]);

  function toggleRole(accountId: string, accountName: string, roleName: string) {
    const key = selectionKey(accountId, roleName);
    setSelections((prev) => {
      const next = new Map(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.set(key, { accountId, accountName, roleName });
      }
      return next;
    });
  }

  async function handleRefresh() {
    setRefreshing(true);
    try {
      const status = await refreshTokenNow();
      onTokenUpdate(status);
    } catch (err) {
      onError(String(err));
    } finally {
      setRefreshing(false);
    }
  }

  async function handleLogout() {
    setLoggingOut(true);
    try {
      const state = await ssoLogout();
      onLogout(state);
    } catch (err) {
      onError(String(err));
      setLoggingOut(false);
    }
  }

  async function handleApply() {
    setApplying(true);
    setAppliedProfiles([]);
    try {
      const effectiveRegion =
        regionChoice === CUSTOM_REGION ? customRegion.trim() : regionChoice;
      const defaultSelection = defaultKey !== NO_DEFAULT ? selections.get(defaultKey) : undefined;
      const defaultProfileName = defaultSelection
        ? computeProfileName(defaultSelection.accountName, defaultSelection.roleName)
        : null;

      const profiles = await applyProfiles(
        Array.from(selections.values()),
        defaultProfileName,
        effectiveRegion,
      );
      setAppliedProfiles(profiles.map((p) => ({ ...p, status: "pending" })));

      for (const profile of profiles) {
        try {
          const identity = await verifyProfile(profile.profileName);
          setAppliedProfiles((prev) =>
            prev.map((p) =>
              p.profileName === profile.profileName
                ? { ...p, status: "ok", resolvedArn: identity.arn }
                : p,
            ),
          );
        } catch (err) {
          setAppliedProfiles((prev) =>
            prev.map((p) =>
              p.profileName === profile.profileName
                ? { ...p, status: "error", errorMessage: String(err) }
                : p,
            ),
          );
        }
      }
    } catch (err) {
      onError(String(err));
    } finally {
      setApplying(false);
    }
  }

  async function handleCopyExports(profileName: string) {
    try {
      const format = detectExportFormat();
      const text = await exportCredentials(profileName, format);
      await copyText(text);
      setCopiedFlash(profileName);
      setTimeout(() => {
        setCopiedFlash((current) => (current === profileName ? null : current));
      }, 1500);
    } catch (err) {
      onError(String(err));
    }
  }

  const defaultOptions = Array.from(selections.entries());
  const firstSelection = Array.from(selections.values())[0];
  const footerProfileName =
    appliedProfiles[0]?.profileName ??
    (firstSelection
      ? computeProfileName(firstSelection.accountName, firstSelection.roleName)
      : "<profile-name>");

  return (
    <div className="screen main-screen">
      <header className="main-header">
        <div className="main-header-title">
          <h1>{sessionName}</h1>
          <span className="expiry-countdown mono">
            {token.valid ? `expires in ${formatDuration(displaySeconds)}` : "expired"}
          </span>
        </div>
        <div className="main-header-actions">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={handleRefresh}
            disabled={refreshing || !token.canRefresh}
          >
            {refreshing ? "Refreshing…" : "Refresh"}
          </button>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={handleLogout}
            disabled={loggingOut}
          >
            {loggingOut ? "Signing out…" : "Sign out"}
          </button>
        </div>
      </header>

      {!cliInstalled && (
        <Banner
          kind="warning"
          message="AWS CLI v2 not found — install it to use the profiles from your terminal."
        />
      )}

      <div className="main-body">
        {accounts === null ? (
          <Spinner label="Loading accounts…" />
        ) : (
          <div className="account-list">
            {accounts.map((account) => (
              <div className="account-card" key={account.accountId}>
                <div className="account-card-info">
                  <div className="account-name">{account.accountName}</div>
                  <div className="account-meta mono">{account.accountId}</div>
                  <div className="account-meta">{account.email}</div>
                </div>
                <div className="role-list">
                  {account.roles.map((role) => {
                    const key = selectionKey(account.accountId, role);
                    return (
                      <label className="role-checkbox" key={key}>
                        <input
                          type="checkbox"
                          checked={selections.has(key)}
                          onChange={() =>
                            toggleRole(account.accountId, account.accountName, role)
                          }
                        />
                        <span className="mono">{role}</span>
                      </label>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="apply-controls">
          <label className="field">
            <span>Profile region</span>
            <select
              value={regionChoice}
              onChange={(e) => setRegionChoice(e.currentTarget.value)}
            >
              {REGION_PRESETS.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
              <option value={CUSTOM_REGION}>Other…</option>
            </select>
            {regionChoice === CUSTOM_REGION && (
              <input
                value={customRegion}
                onChange={(e) => setCustomRegion(e.currentTarget.value)}
                placeholder="e.g. ap-southeast-1"
              />
            )}
          </label>

          <label className="field">
            <span>Default profile</span>
            <select value={defaultKey} onChange={(e) => setDefaultKey(e.currentTarget.value)}>
              <option value={NO_DEFAULT}>(none)</option>
              {defaultOptions.map(([key, sel]) => (
                <option key={key} value={key}>
                  {sel.accountName} / {sel.roleName}
                </option>
              ))}
            </select>
          </label>

          <button
            type="button"
            className="btn btn-primary"
            onClick={handleApply}
            disabled={applying || accounts === null}
          >
            {applying ? "Applying…" : "Apply"}
          </button>
        </div>

        {appliedProfiles.length > 0 && (
          <div className="applied-list">
            {appliedProfiles.map((profile) => (
              <div className="applied-row" key={profile.profileName}>
                <div className="applied-status">
                  {profile.status === "pending" && <Spinner />}
                  {profile.status === "ok" && <span className="status-ok">&#10003;</span>}
                  {profile.status === "error" && <span className="status-error">&#10007;</span>}
                </div>
                <div className="applied-info">
                  <code className="mono profile-name">{profile.profileName}</code>
                  {profile.status === "ok" && (
                    <span className="mono applied-arn">{profile.resolvedArn}</span>
                  )}
                  {profile.status === "error" && (
                    <span className="applied-error">{profile.errorMessage}</span>
                  )}
                </div>
                <div className="applied-actions">
                  <button
                    type="button"
                    className="btn btn-small"
                    onClick={() => handleCopyExports(profile.profileName)}
                  >
                    {copiedFlash === profile.profileName ? "Copied ✓" : "Copy env exports"}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      <footer className="main-footer mono">export AWS_PROFILE={footerProfileName}</footer>
    </div>
  );
}
