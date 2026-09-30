import "./styles/controls.css";
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
    const visible = props && props.visible ? props.visible : {};
    return <>
        <Taskbar />
        {visible.launcher ? <Launcher /> : null}
        {visible.quickSettings ? <QuickSettings /> : null}
        {visible.notifications ? <Notifications /> : null}
        {visible.settings ? <Settings /> : null}
    </>;
}

export default Shell;
