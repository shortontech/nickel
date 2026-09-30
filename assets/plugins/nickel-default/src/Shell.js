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
import { WindowMenu } from "./WindowMenu.js";
import { Taskbar } from "./Taskbar.js";
import { Launcher } from "./Launcher.js";
import { QuickSettings } from "./QuickSettings.js";
import { VolumeOSD } from "./VolumeOSD.js";
import { Notifications } from "./Notifications.js";
import { Settings, SettingsNavigation, SettingControl } from "./Settings.js";
export { WindowMenu, Taskbar, Launcher, QuickSettings, Notifications, VolumeOSD, Settings };
export { SettingsNavigation, SettingControl };
// The shared package host supplies surface visibility; callers may provide snapshots.
export function Shell(props) {
    const WindowMenu = nickel.component("shell.windowMenu");
    const Taskbar = nickel.component('shell.taskbar');
    const Launcher = nickel.component('shell.launcher');
    const QuickSettings = nickel.component('shell.quickSettings');
    const Notifications = nickel.component('shell.notifications');
    const Settings = nickel.component('shell.settings');
    const VolumeOSD = nickel.component('shell.volumeOSD');
    const state = nickel.data.shell || {};
    const visible = props?.visible || state.visible || { [nickel.data.surface?.id === 'quick-settings' ? 'quickSettings' : nickel.data.surface?.id === 'window-menu' ? 'windowMenu' : nickel.data.surface?.id === 'volume-osd' ? 'volumeOSD' : nickel.data.surface?.id]: true };
    const snapshots = props?.snapshots || state.snapshots || {};
    return h(Fragment, null,
        visible.taskbar ? h(Taskbar, { data: snapshots.taskbar }) : null,
        visible.launcher ? h(Launcher, { data: snapshots.launcher }) : null,
        visible.quickSettings ? h(QuickSettings, null) : null,
        visible.notifications ? h(Notifications, { data: snapshots.notifications }) : null,
        visible.windowMenu ? h(WindowMenu, null) : null,
        visible.volumeOSD ? h(VolumeOSD, null) : null,
        visible.settings ? h(Settings, null) : null);
}
export default Shell;
