// Grouping belongs to the shell package; native capabilities retain stable IDs.
export function taskItems(applications, windows) {
    const groups = new Map();
    for (const application of applications.slice().sort((left,right) => (left.pinOrder ?? Number.MAX_SAFE_INTEGER) - (right.pinOrder ?? Number.MAX_SAFE_INTEGER))) {
        if (application.pinned) groups.set(application.id, {id:application.id, name:application.name, icon:application.icon, pinned:true, canPin:application.canPin !== false && application.canLaunch !== false, canLaunch:application.canLaunch !== false, windows:[], active:false});
    }
    for (const window of windows) {
        const id = window.applicationId || `window:${window.id}`;
        if (!groups.has(id)) {
            const application = applications.find(application => application.id === id);
            groups.set(id, {id, name:application?.name || window.title || 'Window', icon:application?.icon, pinned:false, canPin:!!application && application.canPin !== false && application.canLaunch !== false, canLaunch:!!application && application.canLaunch !== false, windows:[], active:false});
        }
        const group = groups.get(id);
        group.windows.push(window);
        group.active ||= window.active;
    }
    return Array.from(groups.values());
}
export function activateTask(item, capabilities) {
    if (!item.windows.length) return item.canLaunch === false ? undefined : capabilities.applications.launch(item.id);
    const active = item.windows.findIndex(window => window.active);
    const window = item.windows[(active + 1) % item.windows.length];
    if (window.canActivate !== false) capabilities.windows.activate(window.id);
}
