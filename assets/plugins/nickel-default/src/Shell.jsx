import "./styles/controls.css";
import "./Appearance.js";
import "./DefaultApps.js";
import "./Displays.js";
import "./Wifi.js";
import "./Bluetooth.js";
import "./Preferences.js";
import "./Plugins.js";
import "./About.js";
import "./OptionalFeatures.js";
import "./KeyboardShortcuts.js";
// @jsx h
import { Taskbar } from "./Taskbar.js";
import { Launcher } from "./Launcher.js";
import { QuickSettings } from "./QuickSettings.js";
import { Notifications } from "./Notifications.js";
import { Settings, SettingsNavigation, SettingControl } from "./Settings.js";

export { Taskbar, Launcher, QuickSettings, Notifications, Settings };
export { SettingsNavigation, SettingControl };

// The shared package host supplies surface visibility; callers may provide snapshots.
export function Shell(props) {
    const Taskbar = nickel.component('shell.taskbar');
    const Launcher = nickel.component('shell.launcher');
    const QuickSettings = nickel.component('shell.quickSettings');
    const Notifications = nickel.component('shell.notifications');
    const Settings = nickel.component('shell.settings');
    const state = nickel.data.shell || {};
    const visible = props?.visible || state.visible || {[nickel.data.surface?.id === 'quick-settings' ? 'quickSettings' : nickel.data.surface?.id]: true};
    const snapshots = props?.snapshots || state.snapshots || {};
    return <>
        {visible.taskbar ? <Taskbar data={snapshots.taskbar} /> : null}
        {visible.launcher ? <Launcher data={snapshots.launcher} /> : null}
        {visible.quickSettings ? <QuickSettings /> : null}
        {visible.notifications ? <Notifications data={snapshots.notifications} /> : null}
        {visible.settings ? <Settings /> : null}
    </>;
}

export default Shell;
