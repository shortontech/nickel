// @jsx h
import { Taskbar } from "./Taskbar.jsx";
import { Launcher } from "./Launcher.jsx";
import { QuickSettings } from "./QuickSettings.jsx";
import { Notifications } from "./Notifications.jsx";
import { Settings, SettingsNavigation, SettingControl } from "./Settings.jsx";

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
