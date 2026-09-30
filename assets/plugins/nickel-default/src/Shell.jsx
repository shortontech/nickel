import "./styles/controls.css";
import "./Appearance.js";
import "./DefaultApps.js";
import "./Displays.js";
import "./Wifi.js";
import "./Bluetooth.js";
import "./Preferences.js";
import "./Plugins.js";
// @jsx h
import { Taskbar } from "./Taskbar.js";
import { Launcher } from "./Launcher.js";
import { QuickSettings } from "./QuickSettings.js";
import { Notifications } from "./Notifications.js";
import { Settings, SettingsNavigation, SettingControl } from "./Settings.js";

export { Taskbar, Launcher, QuickSettings, Notifications, Settings };
export { SettingsNavigation, SettingControl };

// The package host supplies visibility as surface state. Keeping it in props
// makes this draft independent of today's one-host-per-package show/hide path.
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
        {visible.quickSettings ? <QuickSettings data={snapshots.quickSettings} /> : null}
        {visible.notifications ? <Notifications data={snapshots.notifications} /> : null}
        {visible.settings ? <Settings /> : null}
    </>;
}

export default Shell;
