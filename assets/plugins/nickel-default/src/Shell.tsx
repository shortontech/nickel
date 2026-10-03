import "./styles/controls.css";
import "./Appearance.tsx";
import "./DefaultApps.tsx";
import "./Displays.tsx";
import "./Wifi.tsx";
import "./Bluetooth.tsx";
import "./Preferences.tsx";
import "./Plugins.tsx";
import "./About.tsx";
import "./OptionalFeatures.tsx";
import "./KeyboardShortcuts.tsx";
// @jsx h
import { Run } from "./Run.tsx";
import { WindowMenu } from "./WindowMenu.tsx";
import { Taskbar } from "./Taskbar.tsx";
import { Launcher } from "./Launcher.tsx";
import { QuickSettings } from "./QuickSettings.tsx";
import { Preview } from "./Preview.tsx";
import { VolumeOSD } from "./VolumeOSD.tsx";
import { OnScreenKeyboard } from "./OnScreenKeyboard.tsx";
import { Notifications } from "./Notifications.tsx";
import { Settings, SettingsNavigation, SettingControl } from "./Settings.tsx";

export { Run, Preview, WindowMenu, Taskbar, Launcher, QuickSettings, Notifications, VolumeOSD, Settings, OnScreenKeyboard };
export { SettingsNavigation, SettingControl };

// The shared package host supplies surface visibility; callers may provide snapshots.
export function Shell(props) {
    const Run = nickel.component("shell.run");
    const WindowMenu = nickel.component("shell.windowMenu");
    const Taskbar = nickel.component('shell.taskbar');
    const Launcher = nickel.component('shell.launcher');
    const QuickSettings = nickel.component('shell.quickSettings');
    const OnScreenKeyboard = nickel.component('shell.keyboard');
    const Notifications = nickel.component('shell.notifications');
    const Settings = nickel.component('shell.settings');
    const VolumeOSD = nickel.component('shell.volumeOSD');
    const Preview = nickel.component('shell.window-preview');
    const state = nickel.data.shell || {};
    const visible = props?.visible || state.visible || {[nickel.data.surface?.id === 'quick-settings' ? 'quickSettings' : nickel.data.surface?.id === 'window-menu' ? 'windowMenu' : nickel.data.surface?.id === 'volume-osd' ? 'volumeOSD' : nickel.data.surface?.id]: true};
    const snapshots = props?.snapshots || state.snapshots || {};
    return <>
        {visible.run ? <Run /> : null}
        {visible.taskbar ? <Taskbar data={snapshots.taskbar} /> : null}
        {visible.launcher ? <Launcher data={snapshots.launcher} /> : null}
        {visible.quickSettings ? <QuickSettings /> : null}
        {visible.notifications ? <Notifications data={snapshots.notifications} /> : null}
        {visible.windowMenu ? <WindowMenu /> : null}
        {visible.volumeOSD ? <VolumeOSD /> : null}
        {visible["window-preview"] ? <Preview /> : null}
        {visible.keyboard ? <OnScreenKeyboard /> : null}
        {visible.settings ? <Settings /> : null}
    </>;
}

export default Shell;
