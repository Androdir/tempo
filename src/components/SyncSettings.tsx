import { FormEvent, useCallback, useEffect, useState } from "react";
import { getSyncStatus, importHistoryToHub, onSyncStatus, pairWithHub, setAppMode } from "../api";
import type { SyncStatus } from "../types";

export default function SyncSettings() {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [hubUrl, setHubUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const s = await getSyncStatus();
      setStatus(s);
      if (s.hubUrl && !hubUrl) setHubUrl(s.hubUrl);
    } catch (e) {
      setError(String(e));
    }
  }, [hubUrl]);

  useEffect(() => {
    load();
    let un: (() => void) | undefined;
    onSyncStatus(load).then((u) => (un = u));
    const t = window.setInterval(load, 10000);
    return () => {
      un?.();
      window.clearInterval(t);
    };
  }, [load]);

  const hub = status?.mode === "hub";

  async function toggleMode() {
    await setAppMode(hub ? "local" : "hub");
    await load();
  }

  async function pair(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    setMsg(null);
    try {
      await pairWithHub(hubUrl.trim(), secret);
      setSecret("");
      setMsg("Paired ✓ — events will sync to the hub in the background.");
      await load();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function importHistory() {
    try {
      await importHistoryToHub();
      setMsg("Re-queued local history for upload.");
      await load();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <div className="card card-pad section-gap">
      <h2 className="card-title">🛰️ Tempo Hub sync</h2>
      <p className="card-hint">
        Local-only by default. Connect to a Tempo Hub (e.g. on a Raspberry Pi) to share data and
        dashboard across devices. Only event records sync — never your raw database — and events
        buffer locally if the hub is offline.
      </p>

      <div className="sync-row">
        <div>
          <div className="sync-label">Mode</div>
          <div className="card-hint" style={{ margin: 0 }}>
            {hub ? "Uploading events to the hub." : "Everything stays on this device."}
          </div>
        </div>
        <button className={`pill-toggle ${hub ? "on" : ""}`} onClick={toggleMode}>
          {hub ? "Connected to Hub" : "Local-only"}
        </button>
      </div>

      {hub && status && (
        <div className="sync-status">
          <span className={`sync-dot ${status.connected ? "on" : "off"}`} />
          <b>{status.connected ? "Connected" : "Offline"}</b>
          {status.lastSync && ` · last sync ${new Date(status.lastSync).toLocaleTimeString()}`}
          {` · ${status.queued} queued`}
          {!status.paired && " · not paired yet"}
        </div>
      )}

      <form className="folder-form" onSubmit={pair}>
        <input
          className="pf-input"
          placeholder="Hub URL  e.g. http://tempo-pi:7700"
          value={hubUrl}
          onChange={(e) => setHubUrl(e.target.value)}
        />
        <div className="folder-form-row">
          <input
            className="pf-input"
            type="password"
            placeholder="Pairing secret"
            value={secret}
            onChange={(e) => setSecret(e.target.value)}
          />
          <button className="btn btn-primary" type="submit" disabled={busy || !hubUrl.trim() || !secret}>
            {busy ? "Pairing…" : "Pair this device"}
          </button>
          {hub && status?.paired && (
            <button type="button" className="btn" onClick={importHistory}>
              Import history
            </button>
          )}
        </div>
      </form>

      {error && <div className="error-box" style={{ marginTop: 8 }}>{error}</div>}
      {msg && <div className="muted-num" style={{ marginTop: 8, color: "#16a34a" }}>{msg}</div>}
    </div>
  );
}
