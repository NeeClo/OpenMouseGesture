export interface GestureTemplate {
  name: string;
  points: [number, number][];
}

export type TriggerType = "gesture" | "wheel";
export type WheelTrigger = 
  | "wheel_up" 
  | "wheel_down" 
  | "wheel_click"
  | "x1_button"
  | "x2_button"
  | "leftclick_wheel_up"
  | "leftclick_wheel_down";

export interface Action {
  name?: string;
  trigger_type?: TriggerType;
  gesture: string;
  wheel_trigger?: WheelTrigger;
  action_type: "keystroke" | "command" | "url" | "window_operation";
  keystroke?: string;
  modifiers?: string[];
  command?: string;
  url?: string;
  operation?: "minimize" | "maximize" | "close";
  ignore_exe?: string[];
}

export interface Config {
  trajectory: boolean;
  trajectory_color: string;
  ignore_exe: string[];
  actions: Action[];
}

export type TabId = "gestures" | "actions" | "settings" | "licenses" | "info";

export interface HistoryEntry {
  type: "gesture" | "action" | "config";
  action: "add" | "update" | "delete";
  data: unknown;
  previousData?: unknown;
}
