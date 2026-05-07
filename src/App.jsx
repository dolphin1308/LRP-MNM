import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

function App() {
  // ---- 代理控制 ----
  const [port, setPort] = useState(8080);
  const [running, setRunning] = useState(false);

  // ---- 左侧面板 ----
  const [proxyUrl, setProxyUrl] = useState("http://127.0.0.1:8080");
  const [interceptModel, setInterceptModel] = useState("claude-sonnet-4-5");

  // ---- 右侧面板 ----
  const [targetUrl, setTargetUrl] = useState("https://api.deepseek.com/anthropic");
  const [mappings, setMappings] = useState([
    { source: "claude-sonnet-4-5", target: "deepseek-v4-pro" },
  ]);

  // ---- 日志 ----
  const [logVisible, setLogVisible] = useState(true);
  const [logs, setLogs] = useState([]);
  const logRef = useRef(null);

  // ---- 端口变化同步 ----
  useEffect(() => {
    setProxyUrl(`http://127.0.0.1:${port}`);
  }, [port]);

  // ---- 监听后端日志 ----
  useEffect(() => {
    const unlisten = listen("proxy-log", (event) => {
      const entry = event.payload;
      const now = new Date();
      const ts = now.toTimeString().slice(0, 8);
      setLogs((prev) => [...prev, { ...entry, ts }].slice(-500));
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // ---- 日志自动滚到底部 ----
  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [logs]);

  // ---- 启动/停止 ----
  const toggleProxy = useCallback(async () => {
    try {
      if (running) {
        await invoke("stop_proxy");
        setRunning(false);
      } else {
        const mappingObj = {};
        for (const m of mappings) {
          if (m.source.trim() && m.target.trim()) {
            mappingObj[m.source.trim()] = m.target.trim();
          }
        }
        await invoke("start_proxy", {
          request: { port, target_url: targetUrl.trim(), model_mapping: mappingObj },
        });
        setRunning(true);
      }
    } catch (e) {
      console.error("Proxy toggle error:", e);
    }
  }, [running, port, targetUrl, mappings]);

  // ---- 映射行操作 ----
  const addMapping = () => {
    setMappings((prev) => [...prev, { source: "", target: "" }]);
  };

  const updateMapping = (index, field, value) => {
    setMappings((prev) => {
      const next = [...prev];
      next[index] = { ...next[index], [field]: value };
      return next;
    });
    // 如果是第一行的 source 变化，同步回左侧 Intercept Model
    if (index === 0 && field === "source") {
      setInterceptModel(value);
    }
  };

  const removeMapping = (index) => {
    setMappings((prev) => prev.filter((_, i) => i !== index));
  };

  // ---- 日志颜色 ----
  const getLogColor = (level) => {
    switch (level) {
      case "error": return "var(--strawberry)";
      case "warn":  return "var(--lemon)";
      case "map":   return "var(--lavender)";
      case "request": return "var(--blueberry)";
      case "done":  return "var(--mint)";
      default:      return "var(--text-secondary)";
    }
  };

  return (
    <div className="app">
      {/* ======== 顶部控制栏 ======== */}
      <header className="top-bar">
        <span className="port-label">Port</span>
        <input
          type="number"
          className="port-input"
          value={port}
          onChange={(e) => setPort(Number(e.target.value))}
          min={1024}
          max={65535}
          disabled={running}
        />
        <div className="sep" />
        <button
          className={`toggle-btn ${running ? "stop" : "start"}`}
          onClick={toggleProxy}
        >
          {running ? "Stop" : "Start"}
        </button>
        <span className={`status ${running ? "on" : "off"}`}>
          {running ? `Running · port ${port}` : "Stopped"}
        </span>
      </header>

      {/* ======== 中间主区域 ======== */}
      <div className="main">
        {/* ---- 左侧面板 ---- */}
        <section className="panel client-panel">
          <div className="panel-inner">
            <h2 className="panel-title">Client</h2>
            <p className="hint">
              将 Claude 客户端的 BaseURL 设为下方地址，<br />
              API Key 保持客户端原有设置，自动透传。
            </p>

            <label className="field-label">Proxy URL</label>
            <input
              className="field-input readonly"
              value={proxyUrl}
              readOnly
            />

            <label className="field-label" style={{ marginTop: 12 }}>Intercept Model</label>
            <input
              className="field-input"
              value={interceptModel}
              onChange={(e) => {
                const v = e.target.value;
                setInterceptModel(v);
                // 同步到右侧第一行映射的 source
                setMappings((prev) => {
                  if (prev.length === 0) return prev;
                  const next = [...prev];
                  next[0] = { ...next[0], source: v };
                  return next;
                });
              }}
              placeholder="claude-3-5-sonnet-20241022"
            />
          </div>
        </section>

        {/* ---- 右侧面板 ---- */}
        <section className="panel forward-panel">
          <div className="panel-inner">
            <h2 className="panel-title">Forward</h2>

            <label className="field-label">Target API</label>
            <input
              className="field-input"
              value={targetUrl}
              onChange={(e) => setTargetUrl(e.target.value)}
              placeholder="https://api.deepseek.com/anthropic"
              disabled={running}
            />

            <div className="mapping-header">
              <span className="field-label">Model Mapping</span>
              <span className="mapping-hint">client model → target model</span>
            </div>

            <div className="mapping-list">
              {mappings.map((m, i) => (
                <div className="mapping-row" key={i}>
                  <input
                    className="field-input mapping-input"
                    value={m.source}
                    onChange={(e) => updateMapping(i, "source", e.target.value)}
                    placeholder="client model"
                    disabled={running}
                  />
                  <span className="arrow">→</span>
                  <input
                    className="field-input mapping-input"
                    value={m.target}
                    onChange={(e) => updateMapping(i, "target", e.target.value)}
                    placeholder="target model"
                    disabled={running}
                  />
                  <button
                    className="remove-btn"
                    onClick={() => removeMapping(i)}
                    title="Remove"
                    disabled={running}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>

            <button
              className="add-btn"
              onClick={addMapping}
              disabled={running}
            >
              + Add Mapping
            </button>
          </div>
        </section>
      </div>

      {/* ======== 日志区域 ======== */}
      <div className="log-section">
        <div className="log-header">
          <span className="log-title">Log</span>
          <button
            className="log-toggle"
            onClick={() => setLogVisible(!logVisible)}
          >
            {logVisible ? "Hide" : "Show"}
          </button>
        </div>
        {logVisible && (
          <div className="log-body" ref={logRef}>
            {logs.length === 0 ? (
              <span className="log-empty">Waiting for requests...</span>
            ) : (
              logs.map((log, i) => (
                <div key={i} className="log-line">
                  <span className="log-ts">{log.ts}</span>
                  <span style={{ color: getLogColor(log.level) }}>
                    {log.message}
                  </span>
                </div>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
