import { useRef, useState } from "react";
import {
  ssoLoginCancel,
  ssoLoginStart,
  ssoLoginWait,
  type LoginPrompt,
  type TokenStatus,
} from "../api";
import { branding } from "../branding";
import { Spinner } from "./Spinner";

interface LoginScreenProps {
  sessionName: string;
  onSuccess: (status: TokenStatus) => void;
  onError: (message: string) => void;
}

type Phase = "idle" | "starting" | "waiting" | "cancelling";

export function LoginScreen({ sessionName, onSuccess, onError }: LoginScreenProps) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [prompt, setPrompt] = useState<LoginPrompt | null>(null);
  const cancelledRef = useRef(false);

  async function startLogin() {
    setPhase("starting");
    cancelledRef.current = false;
    try {
      const loginPrompt = await ssoLoginStart();
      setPrompt(loginPrompt);
      setPhase("waiting");
      const status = await ssoLoginWait();
      if (cancelledRef.current) return;
      onSuccess(status);
    } catch (err) {
      if (cancelledRef.current) return;
      onError(String(err));
      setPhase("idle");
      setPrompt(null);
    }
  }

  async function cancelLogin() {
    cancelledRef.current = true;
    setPhase("cancelling");
    try {
      await ssoLoginCancel();
    } catch (err) {
      onError(String(err));
    } finally {
      setPhase("idle");
      setPrompt(null);
    }
  }

  return (
    <div className="screen login-screen">
      <h1>{sessionName}</h1>
      <p className="help-text">Sign in to AWS IAM Identity Center to continue.</p>

      {phase === "idle" || phase === "starting" ? (
        <button
          type="button"
          className="btn btn-primary btn-large"
          onClick={startLogin}
          disabled={phase === "starting"}
        >
          {phase === "starting" ? "Starting…" : `Sign in with ${branding.idpLabel}`}
        </button>
      ) : (
        <div className="login-wait">
          {prompt && (
            <>
              <div className="user-code-box">{prompt.userCode}</div>
              <p className="help-text">
                Browser didn't open? Go to{" "}
                <span className="mono">{prompt.verificationUri}</span> and
                enter the code above.
              </p>
            </>
          )}
          <Spinner label="Waiting for confirmation in your browser…" />
          <button
            type="button"
            className="btn btn-secondary"
            onClick={cancelLogin}
            disabled={phase === "cancelling"}
          >
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}
