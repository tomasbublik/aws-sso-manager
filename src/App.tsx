import { useEffect, useState } from "react";
import { getAppState, subscribeTokenStatus, type AppState, type TokenStatus } from "./api";
import { Banner } from "./components/Banner";
import { Spinner } from "./components/Spinner";
import { Wizard } from "./components/Wizard";
import { LoginScreen } from "./components/LoginScreen";
import { MainScreen } from "./components/MainScreen";
import "./App.css";

function App() {
  const [appState, setAppState] = useState<AppState | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getAppState()
      .then((state) => {
        if (!cancelled) setAppState(state);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = subscribeTokenStatus((status: TokenStatus) => {
      setAppState((prev) => (prev ? { ...prev, token: status } : prev));
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  function handleTokenUpdate(status: TokenStatus) {
    setAppState((prev) => (prev ? { ...prev, token: status } : prev));
  }

  return (
    <main className="app-shell">
      {error && <Banner message={error} onDismiss={() => setError(null)} />}

      {appState === null ? (
        <div className="screen">
          <Spinner label="Loading…" />
        </div>
      ) : appState.session === null ? (
        <Wizard onSaved={setAppState} onError={setError} />
      ) : !appState.token || !appState.token.valid ? (
        <LoginScreen
          sessionName={appState.session.name}
          onSuccess={handleTokenUpdate}
          onError={setError}
        />
      ) : (
        <MainScreen
          sessionName={appState.session.name}
          cliInstalled={appState.cliInstalled}
          token={appState.token}
          onTokenUpdate={handleTokenUpdate}
          onLogout={setAppState}
          onError={setError}
        />
      )}
    </main>
  );
}

export default App;
