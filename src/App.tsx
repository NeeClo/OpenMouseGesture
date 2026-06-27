import { useEffect, useCallback } from "react";
import { Tabs } from "./components/common/Tabs";
import { GesturesTab } from "./components/gestures/GesturesTab";
import { ActionsTab } from "./components/actions/ActionsTab";
import { SettingsTab } from "./components/settings/SettingsTab";
import { LicensesTab } from "./components/licenses/LicensesTab";
import { InfoTab } from "./components/info/InfoTab";
import { ValidationErrorDialog } from "./components/common/ValidationErrorDialog";
import { useStore } from "./store/useStore";
import * as api from "./api/commands";
import "./styles/variables.css";
import "./App.css";

function App() {
  const {
    activeTab,
    setActiveTab,
    setGestures,
    setConfig,
    setLoading,
    setError,
    validationError,
    setValidationError,
    undo,
    redo,
    canUndo,
    canRedo,
    gestures,
    actions,
    addGesture,
    deleteGesture,
    updateGesture,
    addAction,
    deleteAction,
    updateAction,
  } = useStore();

  useEffect(() => {
    const loadData = async () => {
      setLoading(true);
      try {
        // 設定ファイルの検証
        const [configValid, gesturesValid] = await Promise.all([
          api.validateConfigFile(),
          api.validateGesturesFile(),
        ]);

        if (!configValid) {
          const [configPath, errorMessage] = await Promise.all([
            api.getConfigFilePath(),
            api.getConfigValidationError(),
          ]);
          setValidationError({
            fileType: "config",
            filePath: configPath,
            errorMessage: errorMessage || "検証エラーが発生しました",
          });
          setLoading(false);
          return;
        }

        if (!gesturesValid) {
          const [gesturesPath, errorMessage] = await Promise.all([
            api.getGesturesFilePath(),
            api.getGesturesValidationError(),
          ]);
          setValidationError({
            fileType: "gestures",
            filePath: gesturesPath,
            errorMessage: errorMessage || "検証エラーが発生しました",
          });
          setLoading(false);
          return;
        }

        const [gesturesData, configData] = await Promise.all([
          api.getGestures(),
          api.getConfig(),
        ]);
        setGestures(gesturesData);
        setConfig(configData);
      } catch (err) {
        setError(err instanceof Error ? err.message : "データの読み込みに失敗しました");
      } finally {
        setLoading(false);
      }
    };

    loadData();
  }, [setGestures, setConfig, setLoading, setError, setValidationError]);

  const handleValidationResolved = async () => {
    setValidationError(null);
    setLoading(true);
    try {
      const [gesturesData, configData] = await Promise.all([
        api.getGestures(),
        api.getConfig(),
      ]);
      setGestures(gesturesData);
      setConfig(configData);
    } catch (err) {
      setError(err instanceof Error ? err.message : "データの読み込みに失敗しました");
    } finally {
      setLoading(false);
    }
  };

  const handleUndo = useCallback(async () => {
    if (!canUndo()) return;

    const entry = undo();
    if (!entry) return;

    try {
      if (entry.action === "add") {
        if (entry.type === "gesture") {
          const data = entry.data as { name: string };
          await api.deleteGesture(data.name);
          deleteGesture(data.name);
        } else if (entry.type === "action") {
          const data = entry.data as { gesture: string };
          await api.deleteAction(data.gesture);
          deleteAction(data.gesture);
        }
      } else if (entry.action === "delete" && entry.previousData) {
        if (entry.type === "gesture") {
          const prev = entry.previousData as { name: string; points: [number, number][] };
          await api.saveGesture(prev.name, prev.points);
          addGesture(prev);
        } else if (entry.type === "action") {
          const prev = entry.previousData as {
            gesture: string;
            action_type: "keystroke" | "command" | "url" | "window_operation";
          };
          await api.addAction(prev);
          addAction(prev);
        }
      } else if (entry.action === "update" && entry.previousData) {
        if (entry.type === "gesture") {
          const prev = entry.previousData as { name: string; points: [number, number][] };
          const curr = entry.data as { name: string };
          await api.updateGesture(curr.name, prev.name, prev.points);
          updateGesture(curr.name, prev);
        } else if (entry.type === "action") {
          const prev = entry.previousData as {
            gesture: string;
            action_type: "keystroke" | "command" | "url" | "window_operation";
          };
          await api.updateAction(prev.gesture, prev);
          updateAction(prev.gesture, prev);
        } else if (entry.type === "config") {
          const prev = entry.previousData as {
            trajectory: boolean;
            trajectory_color: string;
            ignore_exe: string[];
            actions: [];
          };
          await api.saveConfig(prev);
          setConfig(prev);
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "元に戻す操作に失敗しました");
    }
  }, [
    canUndo,
    undo,
    deleteGesture,
    deleteAction,
    addGesture,
    addAction,
    updateGesture,
    updateAction,
    setConfig,
    setError,
  ]);

  const handleRedo = useCallback(async () => {
    if (!canRedo()) return;

    const entry = redo();
    if (!entry) return;

    try {
      if (entry.action === "add") {
        if (entry.type === "gesture") {
          const data = entry.data as { name: string; points: [number, number][] };
          await api.saveGesture(data.name, data.points);
          addGesture(data);
        } else if (entry.type === "action") {
          const data = entry.data as {
            gesture: string;
            action_type: "keystroke" | "command" | "url" | "window_operation";
          };
          await api.addAction(data);
          addAction(data);
        }
      } else if (entry.action === "delete") {
        if (entry.type === "gesture") {
          const data = entry.data as { name: string };
          await api.deleteGesture(data.name);
          deleteGesture(data.name);
        } else if (entry.type === "action") {
          const data = entry.data as { gesture: string };
          await api.deleteAction(data.gesture);
          deleteAction(data.gesture);
        }
      } else if (entry.action === "update") {
        if (entry.type === "gesture") {
          const data = entry.data as { name: string; points: [number, number][] };
          const prev = entry.previousData as { name: string };
          await api.updateGesture(prev.name, data.name, data.points);
          updateGesture(prev.name, data);
        } else if (entry.type === "action") {
          const data = entry.data as {
            gesture: string;
            action_type: "keystroke" | "command" | "url" | "window_operation";
          };
          await api.updateAction(data.gesture, data);
          updateAction(data.gesture, data);
        } else if (entry.type === "config") {
          const data = entry.data as {
            trajectory: boolean;
            trajectory_color: string;
            ignore_exe: string[];
            actions: [];
          };
          await api.saveConfig(data);
          setConfig(data);
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "やり直し操作に失敗しました");
    }
  }, [
    canRedo,
    redo,
    addGesture,
    addAction,
    deleteGesture,
    deleteAction,
    updateGesture,
    updateAction,
    setConfig,
    setError,
  ]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "z") {
        e.preventDefault();
        handleUndo();
      } else if (e.ctrlKey && e.key === "y") {
        e.preventDefault();
        handleRedo();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleUndo, handleRedo]);

  const renderTabContent = () => {
    // 検証エラーがある場合は何も表示しない
    if (validationError) {
      return null;
    }
    
    switch (activeTab) {
      case "gestures":
        return <GesturesTab />;
      case "actions":
        return <ActionsTab />;
      case "settings":
        return <SettingsTab />;
      case "licenses":
        return <LicensesTab />;
      case "info":
        return <InfoTab />;
      default:
        return null;
    }
  };

  return (
    <div className="app">
      {validationError && (
        <ValidationErrorDialog
          fileType={validationError.fileType}
          filePath={validationError.filePath}
          errorMessage={validationError.errorMessage}
          onResolved={handleValidationResolved}
        />
      )}
      <header className="app-header">
        <Tabs activeTab={activeTab} onTabChange={setActiveTab} />
      </header>
      <main className="app-main">{renderTabContent()}</main>
      <footer className="app-footer">
        <span>ジェスチャー: {gestures.length}</span>
        <span>アクション: {actions.length}</span>
      </footer>
    </div>
  );
}

export default App;
