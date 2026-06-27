/*
概要: アプリ全体の状態管理（タブ、ジェスチャー、アクション、設定、履歴）を行うストア。
入出力:
  - 入力: 各UIからの操作（選択・追加・更新・削除・履歴操作）
  - 出力: 最新の状態（ジェスチャー/アクション配列、設定、選択状態、履歴）
具体例:
  - アクション名の変更: `updateAction` にキー（ジェスチャー名 or wheel_トリガー）と更新後データを渡すと、該当要素が置き換わる。
  - アクション削除: `deleteAction` にキーを渡すと、該当要素が配列から除去される。
*/
import { create } from "zustand";
import type { GestureTemplate, Action, Config, TabId, HistoryEntry } from "../types";

interface AppState {
  activeTab: TabId;
  gestures: GestureTemplate[];
  actions: Action[];
  config: Config;
  selectedGesture: string | null;
  selectedAction: string | null;
  history: HistoryEntry[];
  historyIndex: number;
  isLoading: boolean;
  error: string | null;
  validationError: { fileType: "config" | "gestures"; filePath: string; errorMessage: string } | null;

  setActiveTab: (tab: TabId) => void;
  setGestures: (gestures: GestureTemplate[]) => void;
  setActions: (actions: Action[]) => void;
  setConfig: (config: Config) => void;
  setSelectedGesture: (name: string | null) => void;
  setSelectedAction: (gesture: string | null) => void;
  setLoading: (loading: boolean) => void;
  setError: (error: string | null) => void;
  setValidationError: (error: { fileType: "config" | "gestures"; filePath: string; errorMessage: string } | null) => void;

  addGesture: (gesture: GestureTemplate) => void;
  updateGesture: (oldName: string, gesture: GestureTemplate) => void;
  deleteGesture: (name: string) => void;

  addAction: (action: Action) => void;
  updateAction: (gesture: string, action: Action) => void;
  deleteAction: (gesture: string) => void;

  pushHistory: (entry: HistoryEntry) => void;
  undo: () => HistoryEntry | null;
  redo: () => HistoryEntry | null;
  canUndo: () => boolean;
  canRedo: () => boolean;
}

const MAX_HISTORY = 100;

export const useStore = create<AppState>((set, get) => ({
  activeTab: "gestures",
  gestures: [],
  actions: [],
  config: {
    trajectory: true,
    trajectory_color: "#228B22",
    ignore_exe: [],
    actions: [],
  },
  selectedGesture: null,
  selectedAction: null,
  history: [],
  historyIndex: -1,
  isLoading: false,
  error: null,
  validationError: null,

  setActiveTab: (tab) => set({ activeTab: tab }),
  setGestures: (gestures) => set({ gestures }),
  setActions: (actions) => set({ actions }),
  setConfig: (config) => set({ config, actions: config.actions }),
  setSelectedGesture: (name) => set({ selectedGesture: name }),
  setSelectedAction: (gesture) => set({ selectedAction: gesture }),
  setLoading: (loading) => set({ isLoading: loading }),
  setError: (error) => set({ error }),
  setValidationError: (error) => set({ validationError: error }),

  addGesture: (gesture) =>
    set((state) => ({
      gestures: [...state.gestures, gesture],
    })),

  updateGesture: (oldName, gesture) =>
    set((state) => ({
      gestures: state.gestures.map((g) =>
        g.name === oldName ? gesture : g
      ),
    })),

  deleteGesture: (name) =>
    set((state) => ({
      gestures: state.gestures.filter((g) => g.name !== name),
      selectedGesture: state.selectedGesture === name ? null : state.selectedGesture,
    })),

  addAction: (action) =>
    set((state) => ({
      actions: [...state.actions, action],
    })),

  updateAction: (gesture, action) =>
    set((state) => ({
      actions: state.actions.map((a) => {
        if (gesture.startsWith("wheel_")) {
          const wheel_key = gesture.substring(6);
          return a.trigger_type === "wheel" && a.wheel_trigger === wheel_key ? action : a;
        }
        return a.gesture === gesture ? action : a;
      }),
    })),

  deleteAction: (gesture) =>
    set((state) => ({
      actions: state.actions.filter((a) => {
        if (gesture.startsWith("wheel_")) {
          const wheel_key = gesture.substring(6);
          return !(a.trigger_type === "wheel" && a.wheel_trigger === wheel_key);
        }
        return a.gesture !== gesture;
      }),
      selectedAction: state.selectedAction === gesture ? null : state.selectedAction,
    })),

  pushHistory: (entry) =>
    set((state) => {
      const newHistory = state.history.slice(0, state.historyIndex + 1);
      newHistory.push(entry);
      if (newHistory.length > MAX_HISTORY) {
        newHistory.shift();
      }
      return {
        history: newHistory,
        historyIndex: newHistory.length - 1,
      };
    }),

  undo: () => {
    const state = get();
    if (state.historyIndex < 0) return null;
    const entry = state.history[state.historyIndex];
    set({ historyIndex: state.historyIndex - 1 });
    return entry;
  },

  redo: () => {
    const state = get();
    if (state.historyIndex >= state.history.length - 1) return null;
    const entry = state.history[state.historyIndex + 1];
    set({ historyIndex: state.historyIndex + 1 });
    return entry;
  },

  canUndo: () => get().historyIndex >= 0,
  canRedo: () => get().historyIndex < get().history.length - 1,
}));
