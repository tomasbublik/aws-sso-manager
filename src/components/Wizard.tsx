import { useState, type FormEvent } from "react";
import { saveSsoSession, type AppState } from "../api";
import { branding } from "../branding";

interface WizardProps {
  onSaved: (state: AppState) => void;
  onError: (message: string) => void;
}

export function Wizard({ onSaved, onError }: WizardProps) {
  const [name, setName] = useState<string>(branding.defaults.sessionName);
  const [startUrl, setStartUrl] = useState<string>(branding.defaults.startUrl);
  const [ssoRegion, setSsoRegion] = useState<string>(branding.defaults.ssoRegion);
  const [busy, setBusy] = useState(false);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    try {
      const state = await saveSsoSession(name.trim(), startUrl.trim(), ssoRegion.trim());
      onSaved(state);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="screen wizard-screen">
      <h1>Set up your SSO session</h1>
      <p className="help-text">
        These values come from your company's AWS access portal: expand any
        account &rarr; role &rarr; the &ldquo;Access keys&rdquo; dialog shows
        the SSO start URL and region to use here.
      </p>
      <form onSubmit={handleSubmit} className="form">
        <label className="field">
          <span>Session name</span>
          <input
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
            placeholder={branding.defaults.sessionName || "e.g. my-company"}
            required
          />
        </label>
        <label className="field">
          <span>SSO start URL</span>
          <input
            value={startUrl}
            onChange={(e) => setStartUrl(e.currentTarget.value)}
            placeholder={branding.defaults.startUrl || "https://your-org.awsapps.com/start"}
            required
          />
        </label>
        <label className="field">
          <span>SSO region</span>
          <input
            value={ssoRegion}
            onChange={(e) => setSsoRegion(e.currentTarget.value)}
            placeholder={branding.defaults.ssoRegion || "eu-west-1"}
            required
          />
        </label>
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? "Saving…" : "Save & continue"}
        </button>
      </form>
    </div>
  );
}
