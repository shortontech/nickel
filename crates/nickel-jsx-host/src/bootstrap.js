let __windowsStore = {generation:0, snapshot:Object.freeze([])};
let __applicationsStore = {generation:0, snapshot:Object.freeze([])};
let __notificationsStore = {generation:0, snapshot:Object.freeze({notification:null,history:Object.freeze([]),visible:false})};
let __workspacesStore = {generation:0, snapshot:Object.freeze({generation:0,available:false,reason:null,
    writable:false,revision:null,workspaces:Object.freeze([]),activeWorkspace:null,
    operations:Object.freeze({switch:false,create:false,remove:false})})};
let __outputsStore = {generation:0,snapshot:Object.freeze({generation:0,available:false,reason:null,revision:null,outputs:Object.freeze([])})};
let __windowPreviewsStore = {generation:0, encoded:'null', snapshot:Object.freeze({available:false,windows:Object.freeze([])})};
let __windowMenuStore = {generation:0, encoded:'null', snapshot:Object.freeze({targetId:null})};
const NickelStores=Object.freeze({
    windows:__nickelStoreContract('windows',()=>__windowsStore),applications:__nickelStoreContract('applications',()=>__applicationsStore),
    notifications:__nickelStoreContract('notifications',()=>__notificationsStore),workspaces:__nickelStoreContract('workspaces',()=>__workspacesStore),
    outputs:__nickelStoreContract('outputs',()=>__outputsStore),locale:__nickelStoreContract('locale',()=>__localeStore),
    theme:__nickelStoreContract('theme',()=>__themeStore),capabilities:__nickelStoreContract('capabilities',()=>__capabilityStore)
});
function __nickelWindowEqual(left, right) {
    const fields = ['id','applicationId','title','active','minimized','maximized','fullscreen',
        'workspace','output','canActivate','canClose','canMinimize','canMaximize','canFullscreen',
        'canSnap','canMoveToWorkspace','canMoveToOutput'];
    return fields.every(field => Object.is(left[field] ?? null, right[field] ?? null));
}

function __nickelWindowSnapshot(value) {
    if (!Array.isArray(value) || value.length > 128) throw Error('invalid bounded windows snapshot');
    const previous = new Map(__windowsStore.snapshot.map(window => [window.id, window]));
    const seen = new Set();
    return Object.freeze(value.map(window => {
        if (!window || typeof window !== 'object' || Array.isArray(window)
            || typeof window.id !== 'string' || !window.id.length || window.id.length > 256
            || (window.title !== undefined && (typeof window.title !== 'string' || window.title.length > 480))
            || (window.applicationId !== undefined && window.applicationId !== null
                && (typeof window.applicationId !== 'string' || window.applicationId.length > 256))
            || (window.workspace !== undefined && window.workspace !== null
                && (typeof window.workspace !== 'string' || window.workspace.length > 64))
            || (window.output !== undefined && window.output !== null
                && (typeof window.output !== 'string' || window.output.length > 256))
            || seen.has(window.id)) throw Error('invalid public window snapshot');
        seen.add(window.id);
        const copy = Object.freeze({
            id:window.id,
            applicationId:typeof window.applicationId === 'string' ? window.applicationId : null,
            title:typeof window.title === 'string' ? window.title : '',
            active:window.active === true,
            minimized:window.minimized === true,
            maximized:window.maximized === true,
            fullscreen:window.fullscreen === true,
            workspace:typeof window.workspace === 'string' ? window.workspace : null,
            output:typeof window.output === 'string' ? window.output : null,
            canActivate:window.canActivate === true,
            canClose:window.canClose === true,
            canMinimize:window.canMinimize === true,
            canMaximize:window.canMaximize === true,
            canFullscreen:window.canFullscreen === true,
            canSnap:window.canSnap === true,
            canMoveToWorkspace:window.canMoveToWorkspace === true,
            canMoveToOutput:window.canMoveToOutput === true
        });
        const retained = previous.get(copy.id);
        return retained && __nickelWindowEqual(retained, copy) ? retained : copy;
    }));
}

function __nickelValidateWindowsStorePublication() {
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish windows store during a render or event');
}
function __nickelSetWindowsStore(value) {
    __nickelValidateWindowsStorePublication();
    const before=__nickelDirtyComponentCount(), windows = __nickelWindowSnapshot(value);
    const previous = __windowsStore.snapshot;
    if (previous.length === windows.length && previous.every((window, index) => window === windows[index]))
        return false;
    const generation = __windowsStore.generation + 1;
    __windowsStore = {generation, snapshot:windows};
    __nickelNotifyExternalStore('windows');
    __nickelForEachSurfaceHooks((hooks, dirty) => {
        for (const [owner, slots] of hooks) {
            for (const entry of slots) {
                if (entry?.kind !== 'windows-store') continue;
                try {
                    const selected = entry.selector ? entry.selector(windows) : windows;
                    entry.storeError = undefined;
                    if (!Object.is(selected, entry.value)) dirty.add(owner);
                } catch (error) {
                    entry.storeError = error;
                    dirty.add(owner);
                }
            }
        }
    });
    __nickelRecordStoreChange('windows',generation,before); return true;
}

function __nickelImmutableProjection(value) {
    const copy=JSON.parse(JSON.stringify(value));
    const freeze=value=>{if(value&&typeof value==='object'&&!Object.isFrozen(value)){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
    return freeze(copy);
}
function __nickelSetProjectionStore(name,value) {
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish projection store during a render or event');
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error(`invalid ${name} snapshot`);
    const encoded=JSON.stringify(value);
    const current=name==='windowPreviews'?__windowPreviewsStore:name==='windowMenu'?__windowMenuStore:null;
    if(current===null)throw Error('unknown projection store');
    if(current.encoded===encoded)return false;
    const generation=current.generation+1,snapshot=__nickelImmutableProjection(value),next={generation,encoded,snapshot};
    if(name==='windowPreviews')__windowPreviewsStore=next;else __windowMenuStore=next;
    __nickelForEachSurfaceHooks((hooks,dirty)=>{for(const [owner,slots] of hooks)for(const entry of slots){
        if(entry?.kind!==`${name}-store`)continue;
        try{const selected=entry.selector?entry.selector(snapshot):snapshot;entry.storeError=undefined;if(!Object.is(selected,entry.value))dirty.add(owner);}
        catch(error){entry.storeError=error;dirty.add(owner);}
    }});
    return true;
}
function __nickelUseProjectionStore(name,selector) {
    if(__currentComponent===null)throw Error(`${name} hooks require a component`);
    if(selector!==undefined&&typeof selector!=='function')throw TypeError(`${name} selector must be a function`);
    const slot=__hookIndex++,hooks=__componentHooks.get(__currentComponent),normalized=selector??null;
    const store=name==='windowPreviews'?__windowPreviewsStore:name==='windowMenu'?__windowMenuStore:null;
    if(store===null)throw Error('unknown projection store');
    const entry=__nickelSelectedStoreEntry(hooks,slot,`${name}-store`,normalized,name);
    entry.storeError=undefined;
    entry.value=__nickelSelectedValue(entry,normalized,store.snapshot,store.generation,name);
    entry.generation=store.generation;
    return entry.value;
}
function useWindowPreviews(selector){return __nickelUseProjectionStore('windowPreviews',selector);}
function useWindowMenu(selector){return __nickelUseProjectionStore('windowMenu',selector);}

function useWindows(selector) {
    if (__currentComponent === null) throw Error('useWindows requires a component');
    if (selector !== undefined && typeof selector !== 'function') throw TypeError('useWindows selector must be a function');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    const normalized = selector ?? null;
    const entry=__nickelSelectedStoreEntry(hooks,slot,'windows-store',normalized,'windows');
    entry.storeError = undefined;
    entry.value = __nickelSelectedValue(entry,normalized,__windowsStore.snapshot,__windowsStore.generation,'windows');
    entry.generation = __windowsStore.generation;
    return entry.value;
}

const __nickelSelectActiveWindow = windows => windows.find(window => window.active) ?? null;
function useActiveWindow() { return useWindows(__nickelSelectActiveWindow); }

function __nickelApplicationEqual(left, right) {
    return ['id','name','icon','pinned','pinOrder','recentOrder','kind','launchClass','canLaunch','canPin']
        .every(field => Object.is(left[field], right[field]));
}
function __nickelApplicationSnapshot(value) {
    if (!Array.isArray(value) || value.length > 256) throw Error('invalid bounded applications snapshot');
    const previous = new Map(__applicationsStore.snapshot.map(application => [application.id, application]));
    const seen = new Set();
    return Object.freeze(value.map(application => {
        if (!application || typeof application !== 'object' || Array.isArray(application)
            || typeof application.id !== 'string' || !application.id.length || application.id.length > 256
            || typeof application.name !== 'string' || application.name.length > 480
            || typeof application.icon !== 'string' || application.icon.length > 512
            || (application.kind !== 'place' && application.kind !== 'application')
            || !['graphical','terminal','running'].includes(application.launchClass)
            || seen.has(application.id)) throw Error('invalid public application snapshot');
        const nullableOrder = field => application[field] === undefined || application[field] === null ? null
            : Number.isInteger(application[field]) && application[field] >= 0 && application[field] < 256
            ? application[field] : (() => { throw Error(`invalid application ${field}`); })();
        seen.add(application.id);
        const copy = Object.freeze({id:application.id, name:application.name, icon:application.icon,
            pinned:application.pinned === true, pinOrder:nullableOrder('pinOrder'),
            recentOrder:nullableOrder('recentOrder'), kind:application.kind, launchClass:application.launchClass,
            canLaunch:application.canLaunch !== false, canPin:application.canPin !== false});
        const retained = previous.get(copy.id);
        return retained && __nickelApplicationEqual(retained, copy) ? retained : copy;
    }));
}
function __nickelSetApplicationsStore(value) {
    const before=__nickelDirtyComponentCount(), applications = __nickelApplicationSnapshot(value);
    const previous = __applicationsStore.snapshot;
    if (previous.length === applications.length
        && previous.every((application, index) => application === applications[index])) return false;
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish changed applications store during a render or event');
    const generation = __applicationsStore.generation + 1;
    __applicationsStore = {generation, snapshot:applications};
    __nickelNotifyExternalStore('applications');
    __nickelForEachSurfaceHooks((hooks, dirty) => {
        for (const [owner, slots] of hooks) for (const entry of slots) {
            if (entry?.kind !== 'applications-store') continue;
            try {
                const selected = entry.selector ? entry.selector(applications) : applications;
                entry.storeError = undefined;
                if (!Object.is(selected, entry.value)) dirty.add(owner);
            } catch (error) { entry.storeError = error; dirty.add(owner); }
        }
    });
    __nickelRecordStoreChange('applications',generation,before); return true;
}
function useApplications(selector) {
    if (__currentComponent === null) throw Error('useApplications requires a component');
    if (selector !== undefined && typeof selector !== 'function') throw TypeError('useApplications selector must be a function');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    const normalized = selector ?? null;
    const entry=__nickelSelectedStoreEntry(hooks,slot,'applications-store',normalized,'applications');
    entry.storeError = undefined;
    entry.value = __nickelSelectedValue(entry,normalized,__applicationsStore.snapshot,__applicationsStore.generation,'applications');
    entry.generation = __applicationsStore.generation;
    return entry.value;
}

function __nickelNotificationEqual(left, right) {
    return left.id === right.id && left.appName === right.appName && left.summary === right.summary
        && left.body === right.body && left.actions.length === right.actions.length
        && left.actions.every((action, index) => action.key === right.actions[index].key
            && action.label === right.actions[index].label);
}
function __nickelNotificationRecord(value, previous) {
    if (!value || typeof value !== 'object' || Array.isArray(value)
        || !Number.isSafeInteger(value.id) || value.id < 1 || value.id > 0xffffffff
        || typeof value.appName !== 'string' || value.appName.length > 480
        || typeof value.summary !== 'string' || value.summary.length > 1024
        || typeof value.body !== 'string' || value.body.length > 16384
        || !Array.isArray(value.actions) || value.actions.length > 3)
        throw Error('invalid public notification');
    const actions = Object.freeze(value.actions.map(action => {
        if (!action || typeof action !== 'object' || Array.isArray(action)
            || typeof action.key !== 'string' || !action.key.length || action.key.length > 128
            || typeof action.label !== 'string' || action.label.length > 480)
            throw Error('invalid public notification action');
        return Object.freeze({key:action.key,label:action.label});
    }));
    const copy = Object.freeze({id:value.id,appName:value.appName,summary:value.summary,body:value.body,actions});
    return previous && __nickelNotificationEqual(previous, copy) ? previous : copy;
}
function __nickelNotificationSnapshot(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value)
        || !Array.isArray(value.history) || value.history.length > 12)
        throw Error('invalid bounded notifications snapshot');
    const previousRecords = new Map(__notificationsStore.snapshot.history.map(item => [item.id,item]));
    if (__notificationsStore.snapshot.notification)
        previousRecords.set(__notificationsStore.snapshot.notification.id, __notificationsStore.snapshot.notification);
    const history = Object.freeze(value.history.map(item => {
        const record = __nickelNotificationRecord(item, previousRecords.get(item?.id));
        previousRecords.set(record.id, record);
        return record;
    }));
    const notification = value.notification === undefined || value.notification === null ? null
        : __nickelNotificationRecord(value.notification, previousRecords.get(value.notification.id));
    const visible = value.visible === undefined ? notification !== null
        : typeof value.visible === 'boolean' ? value.visible
        : (() => { throw Error('invalid notification visibility'); })();
    return Object.freeze({notification,history,visible});
}
function __nickelSetNotificationsStore(value) {
    const before=__nickelDirtyComponentCount(), snapshot = __nickelNotificationSnapshot(value), previous = __notificationsStore.snapshot;
    if (previous.notification === snapshot.notification && previous.visible === snapshot.visible
        && previous.history.length === snapshot.history.length
        && previous.history.every((item,index) => item === snapshot.history[index])) return false;
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish changed notifications store during a render or event');
    const generation = __notificationsStore.generation + 1;
    __notificationsStore = {generation,snapshot};
    __nickelNotifyExternalStore('notifications');
    __nickelForEachSurfaceHooks((hooks, dirty) => {
        for (const [owner, slots] of hooks) for (const entry of slots) {
            if (entry?.kind !== 'notifications-store') continue;
            try {
                const selected = entry.selector ? entry.selector(snapshot) : snapshot;
                entry.storeError = undefined;
                if (!Object.is(selected, entry.value)) dirty.add(owner);
            } catch (error) { entry.storeError = error; dirty.add(owner); }
        }
    });
    __nickelRecordStoreChange('notifications',generation,before); return true;
}
function useNotifications(selector) {
    if (__currentComponent === null) throw Error('useNotifications requires a component');
    if (selector !== undefined && typeof selector !== 'function') throw TypeError('useNotifications selector must be a function');
    const slot=__hookIndex++, hooks=__componentHooks.get(__currentComponent), normalized=selector??null;
    const entry=__nickelSelectedStoreEntry(hooks,slot,'notifications-store',normalized,'notifications');
    entry.storeError=undefined;
    entry.value=__nickelSelectedValue(entry,normalized,__notificationsStore.snapshot,__notificationsStore.generation,'notifications');
    entry.generation=__notificationsStore.generation;
    return entry.value;
}

function __nickelWorkspaceEqual(left,right) { return left.id===right.id && left.active===right.active; }
function __nickelWorkspaceSnapshot(value,generation) {
    if (!value || typeof value !== 'object' || Array.isArray(value)
        || !Array.isArray(value.workspaces) || value.workspaces.length>32)
        throw Error('invalid bounded workspaces snapshot');
    const previous=new Map(__workspacesStore.snapshot.workspaces.map(workspace=>[workspace.id,workspace])), seen=new Set();
    const workspaces=Object.freeze(value.workspaces.map(workspace=>{
        if (!workspace || typeof workspace!=='object' || Array.isArray(workspace)
            || typeof workspace.id!=='string' || !workspace.id.length || workspace.id.length>64 || seen.has(workspace.id))
            throw Error('invalid public workspace');
        seen.add(workspace.id);
        const copy=Object.freeze({id:workspace.id,active:workspace.active===true}), retained=previous.get(copy.id);
        return retained&&__nickelWorkspaceEqual(retained,copy)?retained:copy;
    }));
    const available=value.available===true;
    const reason=value.reason===undefined||value.reason===null?null
        :typeof value.reason==='string'&&value.reason.length<=480?value.reason
        :(()=>{throw Error('invalid workspace reason')})();
    const revision=value.revision===undefined||value.revision===null?null
        :typeof value.revision==='string'&&value.revision.length<=128?value.revision
        :(()=>{throw Error('invalid workspace revision')})();
    const activeWorkspace=value.activeWorkspace===undefined||value.activeWorkspace===null
        ?workspaces.find(workspace=>workspace.active)?.id??null
        :typeof value.activeWorkspace==='string'&&seen.has(value.activeWorkspace)?value.activeWorkspace
        :(()=>{throw Error('invalid active workspace')})();
    const source=value.operations??{};
    if (!source || typeof source!=='object' || Array.isArray(source)) throw Error('invalid workspace operations');
    const operations=Object.freeze({switch:source.switch===true,create:source.create===true,remove:source.remove===true});
    return Object.freeze({generation,available,reason,writable:operations.switch||operations.create||operations.remove,
        revision,workspaces,activeWorkspace,operations});
}
function __nickelSetWorkspacesStore(value) {
    const before=__nickelDirtyComponentCount(), generation=__workspacesStore.generation+1, snapshot=__nickelWorkspaceSnapshot(value,generation), previous=__workspacesStore.snapshot;
    const unchanged=previous.available===snapshot.available&&previous.reason===snapshot.reason
        &&previous.writable===snapshot.writable&&previous.revision===snapshot.revision
        &&previous.activeWorkspace===snapshot.activeWorkspace
        &&previous.operations.switch===snapshot.operations.switch&&previous.operations.create===snapshot.operations.create
        &&previous.operations.remove===snapshot.operations.remove&&previous.workspaces.length===snapshot.workspaces.length
        &&previous.workspaces.every((workspace,index)=>workspace===snapshot.workspaces[index]);
    if (unchanged) return false;
    if (__pendingRender!==null||__pendingEvent!==null) throw Error('cannot publish changed workspaces store during a render or event');
    const retainedOperations=previous.operations.switch===snapshot.operations.switch&&previous.operations.create===snapshot.operations.create
        &&previous.operations.remove===snapshot.operations.remove?previous.operations:snapshot.operations;
    const retained=Object.freeze({...snapshot,operations:retainedOperations});
    __workspacesStore={generation,snapshot:retained};
    __nickelNotifyExternalStore('workspaces');
    __nickelForEachSurfaceHooks((hooks,dirty)=>{for(const [owner,slots] of hooks)for(const entry of slots){
        if(entry?.kind!=='workspaces-store')continue;
        try{const selected=entry.selector?entry.selector(retained):retained;entry.storeError=undefined;if(!Object.is(selected,entry.value))dirty.add(owner);}
        catch(error){entry.storeError=error;dirty.add(owner);}
    }});__nickelRecordStoreChange('workspaces',generation,before);return true;
}
function useWorkspaces(selector) {
    if(__currentComponent===null)throw Error('useWorkspaces requires a component');
    if(selector!==undefined&&typeof selector!=='function')throw TypeError('useWorkspaces selector must be a function');
    const slot=__hookIndex++,hooks=__componentHooks.get(__currentComponent),normalized=selector??null;
    const entry=__nickelSelectedStoreEntry(hooks,slot,'workspaces-store',normalized,'workspaces');
    entry.storeError=undefined;entry.value=__nickelSelectedValue(entry,normalized,__workspacesStore.snapshot,__workspacesStore.generation,'workspaces');entry.generation=__workspacesStore.generation;return entry.value;
}
const __nickelSelectWorkspace=snapshot=>snapshot.workspaces.find(workspace=>workspace.id===snapshot.activeWorkspace)??null;
function useWorkspace(){return useWorkspaces(__nickelSelectWorkspace);}

function __nickelFreezeOutput(value) {
    if(!value||typeof value!=='object'||Array.isArray(value)||typeof value.name!=='string'||!value.name.length||value.name.length>256
        ||!Array.isArray(value.modes)||value.modes.length>128)throw Error('invalid public output');
    const number=(field,input=value)=>Number.isFinite(input[field])?input[field]:(()=>{throw Error(`invalid output ${field}`)})();
    const rect=field=>{const rect=value[field];if(!rect||typeof rect!=='object'||Array.isArray(rect))throw Error(`invalid output ${field}`);
        return Object.freeze({x:number('x',rect),y:number('y',rect),width:number('width',rect),height:number('height',rect)});};
    const mode=input=>input===null?null:Object.freeze({width:number('width',input),height:number('height',input),refresh_millihz:number('refresh_millihz',input)});
    const transforms=['normal','rotate90','rotate180','rotate270','flipped','flipped90','flipped180','flipped270'];
    if(!transforms.includes(value.transform))throw Error('invalid output transform');
    return Object.freeze({name:value.name,model:typeof value.model==='string'?value.model.slice(0,480):'',geometry:rect('geometry'),work_area:rect('work_area'),
        scale_120:number('scale_120'),transform:value.transform,physical_width_mm:number('physical_width_mm'),physical_height_mm:number('physical_height_mm'),
        primary:value.primary===true,enabled:value.enabled===true,modes:Object.freeze(value.modes.map(mode)),current_mode:mode(value.current_mode??null)});
}
function __nickelOutputEqual(left,right){return JSON.stringify(left)===JSON.stringify(right);}
function __nickelSetOutputsStore(value){
    const before=__nickelDirtyComponentCount();
    if(!value||typeof value!=='object'||Array.isArray(value)||!Array.isArray(value.outputs)||value.outputs.length>32)throw Error('invalid bounded outputs snapshot');
    const previous=__outputsStore.snapshot,byName=new Map(previous.outputs.map(output=>[output.name,output])),seen=new Set();
    const outputs=Object.freeze(value.outputs.map(source=>{const copy=__nickelFreezeOutput(source);if(seen.has(copy.name))throw Error('duplicate output');seen.add(copy.name);
        const prior=byName.get(copy.name);return prior&&__nickelOutputEqual(prior,copy)?prior:copy;}));
    const available=value.available===true,reason=value.reason==null?null:typeof value.reason==='string'&&value.reason.length<=480?value.reason:(()=>{throw Error('invalid output reason')})();
    const revision=value.revision==null?null:typeof value.revision==='string'&&value.revision.length<=128?value.revision:(()=>{throw Error('invalid output revision')})();
    if(previous.available===available&&previous.reason===reason&&previous.revision===revision&&previous.outputs.length===outputs.length&&previous.outputs.every((output,index)=>output===outputs[index]))return false;
    if(__pendingRender!==null||__pendingEvent!==null)throw Error('cannot publish changed outputs store during a render or event');
    const generation=__outputsStore.generation+1,snapshot=Object.freeze({generation,available,reason,revision,outputs});__outputsStore={generation,snapshot};__nickelNotifyExternalStore('outputs');
    __nickelForEachSurfaceHooks((hooks,dirty)=>{for(const [owner,slots] of hooks)for(const entry of slots){if(entry?.kind!=='outputs-store')continue;
        try{const selected=entry.selector?entry.selector(snapshot):snapshot;entry.storeError=undefined;if(!Object.is(selected,entry.value))dirty.add(owner);}catch(error){entry.storeError=error;dirty.add(owner);}}});__nickelRecordStoreChange('outputs',generation,before);return true;
}
function useOutputs(selector){if(__currentComponent===null)throw Error('useOutputs requires a component');if(selector!==undefined&&typeof selector!=='function')throw TypeError('useOutputs selector must be a function');
    const slot=__hookIndex++,hooks=__componentHooks.get(__currentComponent),normalized=selector??null;
    const entry=__nickelSelectedStoreEntry(hooks,slot,'outputs-store',normalized,'outputs');
    entry.storeError=undefined;entry.value=__nickelSelectedValue(entry,normalized,__outputsStore.snapshot,__outputsStore.generation,'outputs');entry.generation=__outputsStore.generation;return entry.value;}
function __nickelRegisterSettingsPageSurface(id, selection) {
    if (typeof selection !== 'string')
        throw Error('invalid Settings page surface selection');
    __nickelRegisterSurfaceApp(id, function App() {
        const {children, ...props} = __nickelHydrateComponentProps(__nickelData.__componentProps);
        return h(__nickelRegisteredPageComponent(selection),
            props, ...(children ?? []));
    });
    return true;
}

const __settings = new Map();
const __settingsPages = new Map();
let __settingsProvider = null;
let __settingsValues = {};
let __settingsSnapshot = {generation: 0, settings: []};
let __settingsPagesSnapshot = {generation: 0, pages: []};
function __settingMetadata(definition, page) {
    if (!definition || typeof definition !== 'object' || Array.isArray(definition))
        throw TypeError('invalid Settings registration');
    if ('providerPackage' in definition) throw TypeError('provider identity is host owned');
    const metadata = {};
    for (const key of Object.keys(definition)) {
        if (key === 'value' || key === 'onChange' || (page && key === 'component')) continue;
        metadata[key] = definition[key];
    }
    if (page) {
        if (typeof definition.component !== 'function') throw TypeError('Settings page component must be a function');
        metadata.label ??= definition.group;
        metadata.component = {module: 'settings-registry', export: 'component'};
    } else {
        if (definition.value !== undefined && typeof definition.value !== 'function') throw TypeError('setting value must be a function');
        if (definition.onChange !== undefined && typeof definition.onChange !== 'function') throw TypeError('setting onChange must be a function');
    }
    const json = JSON.stringify(metadata);
    if (!json || json.length > 1048576) throw RangeError('Settings registration is too large');
    return JSON.parse(json);
}
function __registerSettings(definition, page) {
    if (__settingsProvider !== null) throw Error('Settings registration phase has finished');
    const metadata = __settingMetadata(definition, page);
    if (typeof metadata.id !== 'string' || !metadata.id.length) throw TypeError('Settings registration requires an id');
    const entries = page ? __settingsPages : __settings;
    if (entries.has(metadata.id)) throw Error('duplicate Settings registration id');
    if (entries.size >= (page ? 32 : 128)) throw RangeError('too many Settings registrations');
    entries.set(metadata.id, {metadata, value: definition.value, onChange: definition.onChange, component: definition.component});
}
function registerSetting(definition) { __registerSettings(definition, false); }
function registerSettingsPage(definition) { __registerSettings(definition, true); }
function __nickelSetSettingsValues(values) { __settingsValues = values; }
function __nickelReadSettingsValues() {
    if (__pendingRender !== null || __pendingEvent !== null) throw Error('cannot read Settings during pending transaction');
    const effects = __effects;
    const hooks = new Map(Array.from(__componentHooks, ([path, slots]) => [path, slots.slice()]));
    const values = Array.from(__componentHooks.values(), slots => slots.map(entry => entry.kind === 'ref' ? entry.value.current : entry.value));
    __effects = [];
    try {
        const snapshot = {};
        for (const [id, entry] of __settings) {
            if (!entry.value) continue;
            const value = entry.value();
            if (value === undefined) throw TypeError('Settings getter returned undefined');
            const serialized = JSON.stringify(value);
            if (serialized === undefined) throw TypeError('Settings getter returned a nonserializable value');
            snapshot[id] = JSON.parse(serialized);
        }
        const json = JSON.stringify(snapshot);
        if (json.length > 1048576) throw RangeError('Settings values exceed snapshot limit');
        if (__effects.length) throw Error('Settings getter emitted effects');
        return json;
    } finally {
        __nickelRestoreHooks(hooks, values, 0);
        __effects = effects;
    }
}
function __nickelSettingsMetadata() {
    return JSON.stringify({settings: Array.from(__settings.values(), entry => entry.metadata), pages: Array.from(__settingsPages.values(), entry => entry.metadata)});
}
function __nickelSetSettingsRegistry(provider, settings, pages) {
    if (provider !== __settingsProvider
        || JSON.stringify(settings) !== JSON.stringify(__settingsSnapshot)
        || JSON.stringify(pages) !== JSON.stringify(__settingsPagesSnapshot))
        __nickelDirtyResourceConsumers('__settingsRegistry');
    __settingsProvider = provider;
    __settingsSnapshot = settings;
    __settingsPagesSnapshot = pages;
}
function __readSettings(snapshot, page) {
    const result = JSON.parse(JSON.stringify(snapshot));
    const entries = page ? result.pages : result.settings;
    for (const entry of entries) {
        if (entry.providerPackage !== __settingsProvider) {
            if (!page) {
                entry.value = () => { const values = __settingsValues[entry.providerPackage]; return values && Object.prototype.hasOwnProperty.call(values, entry.id) ? JSON.parse(JSON.stringify(values[entry.id])) : entry.defaultValue; };
                entry.onChange = value => nickel.request({type:'settings.invoke',provider:entry.providerPackage,id:entry.id,value});
            }
            continue;
        }
        const live = (page ? __settingsPages : __settings).get(entry.id);
        if (!live) continue;
        if (page) entry.component = live.component;
        else {
            entry.value = live.value || (() => entry.defaultValue);
            if (live.onChange) entry.onChange = value => __nickelInvokeSetting(entry.providerPackage, entry.id, value);
        }
    }
    return result;
}
function readPluginSettings() { __nickelMarkResourceRead('__settingsRegistry'); return __readSettings(__settingsSnapshot, false); }
function readSettingsPages() { __nickelMarkResourceRead('__settingsRegistry'); return __readSettings(__settingsPagesSnapshot, true); }
function readPluginSettingsPages() { return readSettingsPages(); }
function __nickelRegisteredPageComponent(id) {
    const entry = __settingsPages.get(id);
    if (!entry || typeof entry.component !== 'function') throw Error('Settings page is unavailable');
    return entry.component;
}
function __nickelInvokeSetting(provider, id, value) {
    if (provider !== __settingsProvider || !__settingsSnapshot.settings.some(entry => entry.providerPackage === provider && entry.id === id))
        throw Error('Settings provider is disabled or unavailable');
    const entry = __settings.get(id);
    if (!entry || typeof entry.onChange !== 'function') throw Error('setting has no change handler');
    entry.onChange(value);
}
function __nickelRetireSettings() {
    __settings.clear();
    __settingsPages.clear();
    __settingsProvider = '';
    __settingsValues = {};
    __settingsSnapshot = {generation: 0, settings: []};
    __settingsPagesSnapshot = {generation: 0, pages: []};
}

function __nickelConnectivityEffect(resource, operation, value, identity = false) {
    if (!identity && typeof value !== 'boolean') throw TypeError('invalid connectivity value');
    const snapshot = __nickelData[resource];
    if (!snapshot || !snapshot.available || !snapshot.operations?.[operation]) throw Error('connectivity operation is unavailable');
    const effect = {type:resource + '.' + operation, revision:snapshot.revision};
    effect[identity ? 'id' : 'value'] = value;
    __effects.push(effect);
}
function __nickelAppearanceEffect(resource, change) {
    const snapshot = nickel[resource].get();
    if (!snapshot.available || !Number.isSafeInteger(snapshot.generation) || snapshot.generation < 1) throw Error(resource + ' capability is unavailable');
    if (snapshot.writable !== true) throw Error(resource + ' control capability is unavailable');
    const transaction = {generation:snapshot.generation, prior:snapshot.configured};
    transaction[resource === 'appearance' ? 'requested' : 'change'] = JSON.parse(JSON.stringify(change));
    __effects.push({type:resource === 'appearance' ? 'appearance.set' : 'wallpaper.change', transaction});
}
function __nickelSessionAction(action) {
    const snapshot = nickel.session.get();
    if (snapshot.locked || !snapshot.support[action] || typeof snapshot.revision !== 'string' || !snapshot.revision.length)
        throw Error('session operation is unavailable');
    __effects.push({type:'session.perform',revision:snapshot.revision,action});
}

function __nickelFeatureEffect(operation, values = {}) {
    const snapshot = __nickelData.features;
    if (!snapshot?.available || !snapshot.operations?.[operation]) throw Error('feature operation is unavailable');
    __effects.push({type:'features.' + operation, revision:snapshot.revision, ...values});
}
function __nickelDecideShell(token, revision, confirm) {
            const snapshot = __nickelResource('plugins', {available:false,writable:false});
            const preview = snapshot.shellPreview;
            if (!snapshot.available || !snapshot.writable || !preview || typeof token !== 'string' || token !== preview.token || !preview[confirm ? 'canConfirm' : 'canRevert']) throw Error('shell preview decision is unavailable');
            if (typeof revision !== 'string' || revision !== snapshot.revision) throw Error('plugin inventory is stale');
            __effects.push({type:confirm ? 'plugins.confirmShell' : 'plugins.revertShell', token, revision});
}
function __nickelWorkspaceEffect(operation,id) {
    const snapshot = nickel.workspaces.get();
    if (!snapshot.available || !snapshot.operations[operation]) throw Error('workspace operation unavailable');
    const effect = {type:'workspaces.'+operation,revision:snapshot.revision};
    if (operation !== 'create') {
        id=__nickelIdentity(id);
        if (!snapshot.workspaces.some(workspace=>workspace.id===id)) throw Error('unknown workspace');
        effect.id=id;
    }
    __effects.push(effect);
}
// Nickel-owned shell service clients. Evaluated only by Nickel host constructors.
__nickelPublicRuntimeGlobals = Object.freeze([
    ...__nickelPublicRuntimeGlobals, 'nickel'
]);
const nickel = Object.freeze({
    shortcuts: Object.freeze({
        get() { return __nickelResource('shortcuts', {available:false,editable:false,shortcuts:[],reason:'Shortcut read capability is unavailable'}); }
    }),
    features: Object.freeze({
        get() { return __nickelResource('features', {available:false,operations:{},keyboard:{},codex:{},reason:'Feature read capability is unavailable'}); },
        setKeyboardMode(mode) {
            if (!['automatic','enabled','disabled'].includes(mode)) throw TypeError('invalid keyboard mode');
            __nickelFeatureEffect('setKeyboardMode', {mode});
        },
        setCodexEnabled(enabled, confirmed = false) {
            if (typeof enabled !== 'boolean' || typeof confirmed !== 'boolean') throw TypeError('invalid feature enablement');
            __nickelFeatureEffect('setCodexEnabled', {enabled,confirmed});
        },
        retryCodex() { __nickelFeatureEffect('retryCodex'); }
    }),
    appearance: Object.freeze({
        get() { return __nickelResource('appearance', {available:false,reason:'Appearance read capability is unavailable'}); },
        set(preferences) { __nickelAppearanceEffect('appearance', preferences); }
    }),
    wallpaper: Object.freeze({
        get() { return __nickelResource('wallpaper', {available:false,reason:'Wallpaper read capability is unavailable',images:[]}); },
        listImages() { return this.get().images; },
        setPosition(position) { __nickelAppearanceEffect('wallpaper', {kind:'set_position',position}); },
        resetCustomImage() { __nickelAppearanceEffect('wallpaper', {kind:'reset_custom_image'}); },
        chooseImage() {
            if (arguments.length) throw TypeError('chooseImage takes no arguments');
            const snapshot = this.get();
            if (!snapshot.available || snapshot.writable !== true || !Number.isSafeInteger(snapshot.generation) || snapshot.generation < 1 || !snapshot.chooser || snapshot.chooser.available !== true || snapshot.chooser.pending) throw Error('Native image chooser is unavailable');
            __effects.push({type:'wallpaper.chooseImage',transaction:{generation:snapshot.generation,prior:JSON.parse(JSON.stringify(snapshot.configured))}});
        },
        selectImage(id) { __nickelAppearanceEffect('wallpaper', {kind:'select_approved_image',image_id:__nickelIdentity(id)}); }
    }),
    system: Object.freeze({
        get() { return __nickelResource('system', {available:false,version:null,platform:null,architecture:null}); }
    }),
    session: Object.freeze({
        get() { return __nickelResource('session', {revision:'',account:null,locked:false,support:{lock:false,logout:false,suspend:false,reboot:false,powerOff:false,restartShell:false}}); },
        lock() { __nickelSessionAction('lock'); },
        logout() { __nickelSessionAction('logout'); },
        suspend() { __nickelSessionAction('suspend'); },
        reboot() { __nickelSessionAction('reboot'); },
        powerOff() { __nickelSessionAction('powerOff'); },
        restartShell() { __nickelSessionAction('restartShell'); }
    }),
    audio: Object.freeze({
        get() { return __nickelResource('audio', {available:false,muted:false,percent:0,devices:[]}); },
        outputs() { return this.get().devices; },
        setVolume(percent) {
            if (!Number.isInteger(percent) || percent < 0 || percent > 100) throw TypeError('volume must be an integer from 0 to 100');
            __effects.push({type:'control-action',action:'audio-volume',value:percent});
        },
        setMuted(muted) {
            if (typeof muted !== 'boolean') throw TypeError('muted must be boolean');
            __effects.push({type:'control-action',action:'audio-mute',value:muted});
        },
        selectOutput(id) { __effects.push({type:'control-action',action:'audio-device',value:__nickelIdentity(id)}); }
    }),
    run: Object.freeze({
        get() { return __nickelResource('run', {available:false, status:null}); },
        execute(command, expectedRevision) {
            if (typeof command !== 'string' || !command.trim() || Array.from(command.trim()).length > 4096 || command.includes('\0')) throw TypeError('invalid Run command');
            const current = __nickelResource('run', {available:false});
            const revision = expectedRevision === undefined ? current.revision : expectedRevision;
            if (!current.available || typeof revision !== 'string' || !revision.length || revision.length > 128) throw Error('Run unavailable');
            __effects.push({type:'run.execute', command:command.trim(), revision});
        }
    }),
    projects: Object.freeze({
        show() { __effects.push({type:'projects.show'}); },
        toggle() { __effects.push({type:'projects.toggle'}); }
    }),
    keyboard: Object.freeze({
        get() { return __nickelResource('keyboard', {available:false,generation:0,rows:[],recipientAvailable:false,operations:{}}); },
        toggle() { __effects.push({type:'keyboard.toggle'}); },
        press(id) { if(typeof id !== 'string' || !id || id.length > 64) throw TypeError('invalid keyboard key'); __effects.push({type:'keyboard.press',id,generation:this.get().generation}); },
        hide() { __effects.push({type:'keyboard.hide',generation:this.get().generation}); },
        toggleDock() { __effects.push({type:'keyboard.toggleDock',generation:this.get().generation}); },
        holdModifiers() { __effects.push({type:'keyboard.holdModifiers',generation:this.get().generation}); },
        resize(delta) { if(delta !== -32 && delta !== 32) throw TypeError('invalid keyboard resize'); __effects.push({type:'keyboard.resize',delta,generation:this.get().generation}); }
    }),
    clock: Object.freeze({
        get() { return __nickelResource('clock', {unixMilliseconds:Date.now(),utcOffsetMinutes:0}); }
    }),
    notifications: Object.freeze({
        get() { return __nickelResource('notifications', {notification:null,history:[]}); },
        invoke(id, key) {
            if (!Number.isSafeInteger(id) || id < 1 || id > 4294967295) throw TypeError('invalid notification identity');
            key = __nickelIdentity(key);
            if (key.length > 128) throw TypeError('invalid notification action');
            __effects.push({type:'notifications.invoke',id,key});
        },
        dismiss(id) {
            if (!Number.isSafeInteger(id) || id < 1 || id > 4294967295) throw TypeError('invalid notification identity');
            __effects.push({type:'notifications.dismiss',id});
        }
    }),
    tray: Object.freeze({
        list() { return __nickelResource('tray', []); },
        activate(id) { __effects.push({type:'tray.activate',id:__nickelIdentity(id)}); },
        contextMenu(id) { __effects.push({type:'tray.contextMenu',id:__nickelIdentity(id)}); }
    }),
    windowPreviews: Object.freeze({
        get() { return __nickelResource('windowPreviews', {available:false, windows:[]}); },
        activate(id, revision) { __effects.push({type:'windowPreviews.action', action:'activate', window:__nickelIdentity(id), revision:__nickelIdentity(revision)}); },
        close(id, revision) { __effects.push({type:'windowPreviews.action', action:'close', window:__nickelIdentity(id), revision:__nickelIdentity(revision)}); },
        openMenu(id, revision) { __effects.push({type:'windowPreviews.action', action:'menu', window:__nickelIdentity(id), revision:__nickelIdentity(revision)}); }
    }),
    windows: Object.freeze({
        list() { return __nickelResource('windows', []); },
        activate(id) { __effects.push({type:'windows.focus',id:__nickelIdentity(id)}); },
        close(id) { __effects.push({type:'windows.close',id:__nickelIdentity(id)}); },
        menu() { return __nickelResource('windowMenu', {targetId:null}); },
        dismissMenu(options) { __effects.push({type:'windows.dismissMenu', restoreFocus: options?.restoreFocus ?? true}); },
        showMenu(id) { __effects.push({type:'windows.showMenu',id:__nickelIdentity(id)}); },
        minimize(id) { __effects.push({type:'windows.minimize',id:__nickelIdentity(id)}); },
        maximize(id) { __effects.push({type:'windows.maximize',id:__nickelIdentity(id)}); },
        restore(id) { __effects.push({type:'windows.restore',id:__nickelIdentity(id)}); },
        toggleMaximize(id) { __effects.push({type:'windows.toggleMaximize',id:__nickelIdentity(id)}); },
        toggleFullscreen(id) { __effects.push({type:'windows.toggleFullscreen',id:__nickelIdentity(id)}); },
        snapLeading(id) { __effects.push({type:'windows.snapLeading',id:__nickelIdentity(id)}); },
        snapTrailing(id) { __effects.push({type:'windows.snapTrailing',id:__nickelIdentity(id)}); },
        destinations() { return __nickelResource('windowDestinations', {workspaces:[],outputs:[]}); },
        moveToWorkspace(id, workspace) { __effects.push({type:'windows.moveToWorkspace',id:__nickelIdentity(id),destination:__nickelIdentity(workspace)}); },
        moveToOutput(id, output) { __effects.push({type:'windows.moveToOutput',id:__nickelIdentity(id),destination:String(output)}); }
    }),
    applications: Object.freeze({
        list() { return __nickelResource('applications', []); },
        search(query) {
            if (typeof query !== 'string' || Array.from(query).length > 512 || query.includes('\0')) throw TypeError('invalid application search query');
            __effects.push({type:'applications.search',query});
        },
        searchResults() { return __nickelResource('applicationSearch', {available:false, query:'', results:[], total:0, reason:'Application search is unavailable'}); },
        retryPinSave() { __effects.push({type:'applications-retry-pin-save'}); },
        launch(id) { __effects.push({type:'applications.launch',id:__nickelIdentity(id)}); },
        togglePin(id) { __effects.push({type:'applications.togglePin',id:__nickelIdentity(id)}); },
        movePin(id, direction) {
            if (direction !== -1 && direction !== 1) throw TypeError('pin direction must be -1 or 1');
            __effects.push({type:'applications.movePin',id:__nickelIdentity(id),direction});
        }
    }),
    surfaces: Object.freeze({
        show(id) { __effects.push({type:'surface.show',surfaceId:__nickelIdentity(id)}); },
        hide(id) { __effects.push({type:'surface.hide',surfaceId:__nickelIdentity(id)}); },
        focus(id) { __effects.push({type:'surface.focus',surfaceId:__nickelIdentity(id)}); },
        setPlacement(id, {anchor, offsetX = 0, offsetY = 0}) {
            if (typeof anchor !== 'string' || !Number.isInteger(offsetX) || !Number.isInteger(offsetY)
                || Math.abs(offsetX) > 8192 || Math.abs(offsetY) > 8192)
                throw TypeError('invalid surface placement');
            __effects.push({type:'surface.setPlacement',surfaceId:__nickelIdentity(id),anchor,offsetX,offsetY});
        }
    }),
    // Ordinary package validation has no installed composition catalog. The
    // native composed host replaces this client with resolved owned entries.
    contributions(collection) {
        if (typeof collection !== 'string') throw TypeError('invalid contribution collection');
        return Object.freeze([]);
    },
    component(contract) {
        const component = __nickelPublicComponents.get(contract);
        if (!component) throw Error(`unknown public component ${contract}`);
        return component;
    },
    preferences: Object.freeze({
        get() { return __nickelResource('preferences', {available:false,writable:false,reason:'Preferences read capability is unavailable'}); },
        set(patch) {
            const snapshot = this.get();
            if (!snapshot.available || !snapshot.writable) throw Error('preferences write capability is unavailable');
            if (!patch || typeof patch !== 'object' || Array.isArray(patch) || !Object.keys(patch).length || Object.keys(patch).some(key => !Object.prototype.hasOwnProperty.call(snapshot.configured, key))) throw TypeError('unknown preference fields');
            const requested = Object.assign({}, snapshot.configured, JSON.parse(JSON.stringify(patch)));
            __effects.push({type:'preferences.set',transaction:{revision:snapshot.revision,prior:snapshot.configured,requested,changedFields:Object.keys(patch)}});
        }
    }),
    plugins: Object.freeze({
        get() { return __nickelResource('plugins', {available:false,writable:false,reason:'Plugin read capability is unavailable',plugins:[],lastResult:null}); },
        list() { return this.get().plugins; },
        enable(id, revision) { this.setEnabled(id, true, revision); },
        disable(id, revision) { this.setEnabled(id, false, revision); },
        selectShell(id, revision) {
            id = __nickelIdentity(id);
            const snapshot = this.get();
            const plugin = snapshot.plugins.find(plugin => plugin.id === id);
            if (!snapshot.available || !snapshot.writable || snapshot.shellPreview || !plugin || !plugin.shell) throw Error('shell selection is unavailable');
            if (typeof revision !== 'string' || revision !== snapshot.revision) throw Error('plugin inventory is stale');
            __effects.push({type:'plugins.selectShell',id,revision});
        },
        confirmShell(token, revision) { __nickelDecideShell(token, revision, true); },
        revertShell(token, revision) { __nickelDecideShell(token, revision, false); },
        setSetting(id, key, value, revision) {
            id = __nickelIdentity(id); key = __nickelIdentity(key);
            const snapshot = this.get();
            const plugin = snapshot.plugins.find(plugin => plugin.id === id);
            const setting = plugin?.settings?.find(setting => setting.id === key);
            if (!snapshot.available || !snapshot.writable || !setting) throw Error('plugin setting management is unavailable');
            if (typeof revision !== 'string' || revision !== snapshot.revision) throw Error('plugin inventory is stale');
            if (!['boolean','number','string'].includes(typeof value)) throw TypeError('invalid plugin setting value');
            __effects.push({type:'plugins.setSetting',id,key,revision,priorValue:setting.value,value});
        },
        setEnabled(id, enabled, revision) {
            id = __nickelIdentity(id);
            const snapshot = this.get();
            const plugin = snapshot.plugins.find(plugin => plugin.id === id);
            if (!snapshot.available || !snapshot.writable || !plugin) throw Error('plugin management is unavailable');
            if (typeof revision !== 'string' || revision !== snapshot.revision) throw Error('plugin inventory is stale');
            if (typeof enabled !== 'boolean') throw TypeError('invalid plugin state');
            __effects.push({type:enabled ? 'plugins.enable' : 'plugins.disable',id,revision,priorEnabled:plugin.enabled});
        }
    }),
    associations: Object.freeze({
        get() { return __nickelResource('associations', {available:false,reason:'Associations read capability is unavailable',targets:[],operations:{},lastResult:null}); },
        list() { return this.get().targets; },
        getHandlers(targetId) {
            const target = this.list().find(target => target.id === __nickelIdentity(targetId));
            if (!target) throw Error('association target is unavailable');
            return {revision:this.get().revision,target,handlers:target.handlers};
        },
        setDefault(targetId, handlerId, expectedRevision) {
            const snapshot = this.get();
            const target = snapshot.targets.find(target => target.id === __nickelIdentity(targetId));
            handlerId = __nickelIdentity(handlerId);
            if (typeof expectedRevision !== 'string' || !/^[1-9][0-9]*$/.test(expectedRevision) || expectedRevision !== snapshot.revision)
                throw Error('association snapshot is stale');
            if (!snapshot.available || !target?.canSetDefault || target.protected)
                throw Error('association changes are unavailable or protected');
            const handler = target.handlers.find(handler => handler.id === handlerId);
            if (!handler || handler.protected) throw Error('association handler is unavailable or protected');
            __effects.push({type:'associations.setDefault',targetId,handlerId,revision:expectedRevision,expectedHandlerId:target.effectiveHandlerId});
        },
        openSystemSettings() { __effects.push({type:'associations.openSystemSettings'}); }
    }),
    wifi: Object.freeze({
        get() { return __nickelResource('wifi', {available:false, reason:'Wi-Fi read capability is unavailable', enabled:false, adaptersAvailable:false, adapters:[], networks:[], operations:{}}); },
        listNetworks() { return this.get().networks; },
        setEnabled(value) { __nickelConnectivityEffect('wifi', 'setEnabled', value); },
        connect(id) { __nickelConnectivityEffect('wifi', 'connect', __nickelIdentity(id), true); },
        disconnect(id) { __nickelConnectivityEffect('wifi', 'disconnect', __nickelIdentity(id), true); }
    }),
    bluetooth: Object.freeze({
        get() { return __nickelResource('bluetooth', {available:false, reason:'Bluetooth read capability is unavailable', adapterName:'', powered:false, discovering:false, devices:[], operations:{}}); },
        listDevices() { return this.get().devices; },
        setPowered(value) { __nickelConnectivityEffect('bluetooth', 'setPowered', value); },
        setDiscovery(value) { __nickelConnectivityEffect('bluetooth', 'setDiscovery', value); },
        connect(id) { __nickelConnectivityEffect('bluetooth', 'connect', __nickelIdentity(id), true); },
        disconnect(id) { __nickelConnectivityEffect('bluetooth', 'disconnect', __nickelIdentity(id), true); },
        pair(id) { __nickelConnectivityEffect('bluetooth', 'pair', __nickelIdentity(id), true); }
    }),
    registerSetting, registerSettingsPage, readPluginSettings, readSettingsPages, readPluginSettingsPages,
    request(effect) { __effects.push(effect); },
    openDialog(id) { __effects.push(`open-dialog:${id}`); },
    openMenu(id) { __effects.push(`open-menu:${id}`); },
    workspaces: Object.freeze({
        get() { return __nickelResource('workspaces',{available:false,workspaces:[],operations:{}}); },
        switch(id) { __nickelWorkspaceEffect('switch',id); },
        create() { __nickelWorkspaceEffect('create'); },
        remove(id) { __nickelWorkspaceEffect('remove',id); }
    }),
    desktop: Object.freeze({
        get() { return __nickelResource('desktop',{available:false,operations:{}}); },
        toggleShowDesktop() { if (!this.get().operations.toggleShowDesktop) throw Error('show desktop unavailable'); __effects.push({type:'desktop.toggleShowDesktop'}); }
    }),
    displays: Object.freeze({
        get() {
            __nickelMarkResourceRead('displays');
            const snapshot = __nickelData.displays;
            return snapshot === undefined ? undefined : JSON.parse(JSON.stringify(snapshot));
        },
        previewProjection(mode) {
            const snapshot = this.get();
            if (!snapshot?.projectionModes?.some(entry=>entry.id===mode) || snapshot.pending_confirmation) throw Error('display projection unavailable');
            __effects.push({type:'displays.previewProjection',mode,revision:snapshot.revision});
        },
        getApplicationScale() { return __nickelResource('displays', {}).application_scale || {available:false,reason:'Application scale capability is unavailable'}; },
        setApplicationScale(policy, expectedRevision) {
            const snapshot = this.getApplicationScale();
            const revision = expectedRevision === undefined ? snapshot.revision : expectedRevision;
            if (snapshot.available !== true || typeof revision !== 'string' || revision.length !== 16 || revision !== snapshot.revision) throw Error('application scale observation is unavailable or stale');
            if (!policy || typeof policy !== 'object' || Array.isArray(policy)) throw TypeError('invalid application scale policy');
            if (policy.policy === 'custom') {
                if (!Number.isInteger(policy.scale_120) || !snapshot.supported_scales?.includes(policy.scale_120)) throw RangeError('unsupported custom application scale');
            } else if (policy.policy !== 'follow' && policy.policy !== 'unchanged') throw TypeError('invalid application scale policy');
            __effects.push({type:'displays.setApplicationScale',revision,policy:JSON.parse(JSON.stringify(policy))});
        },
        identify(expectedRevision) {
            const snapshot = this.get();
            const revision = expectedRevision === undefined ? snapshot?.revision : expectedRevision;
            if (snapshot?.operations?.identify !== true || typeof revision !== 'string' || revision.length !== 16 || revision !== snapshot.revision) throw Error('display identification is unavailable or stale');
            __effects.push({type:'displays.identify',revision});
        },
        setLayout(layout, expectedRevision) {
            if (layout === null || typeof layout !== 'object' || Array.isArray(layout)
                || typeof layout.primary !== 'string'
                || !Array.isArray(layout.placements)
                || layout.placements.length < 1 || layout.placements.length > 32)
                throw TypeError('invalid display layout');
            const snapshot = this.get();
            const revision = expectedRevision === undefined ? snapshot?.revision : expectedRevision;
            if (snapshot?.available !== true || typeof revision !== 'string' || revision.length !== 16 || revision !== snapshot.revision) throw Error('display observation is unavailable or stale');
            __effects.push({type: 'displays.setLayout', revision, layout: JSON.parse(JSON.stringify(layout))});
        },
        confirm() { __effects.push({type: 'displays.confirm'}); },
        revert() { __effects.push({type: 'displays.revert'}); }
    }),
    get data() { return __nickelData; }
});


__nickelPublicRuntimeGlobals = Object.freeze([...__nickelPublicRuntimeGlobals,
    'registerSetting','registerSettingsPage','readPluginSettings','readSettingsPages','readPluginSettingsPages']);
__twinkleRegisterCheckpointParticipant({
    capture(copy) { return {values:copy(__settingsValues),settings:copy(__settingsSnapshot),pages:copy(__settingsPagesSnapshot)}; },
    restore(state,original) { __settingsValues=original(state.values);__settingsSnapshot=original(state.settings);__settingsPagesSnapshot=original(state.pages); }
});

__nickelPublicRuntimeGlobals=Object.freeze([...__nickelPublicRuntimeGlobals,'NickelStores','useWindows','useActiveWindow','useWindowPreviews','useWindowMenu','useApplications','useNotifications','useWorkspaces','useWorkspace','useOutputs']);

__twinkleOutputResolver = name => { const topology=useOutputs(); return name===null?null:topology.outputs.find(output=>output.name===name)??null; };

function useCapability(name) { return useHostCapability(name); }
__nickelPublicRuntimeGlobals=Object.freeze([...__nickelPublicRuntimeGlobals,'useCapability']);
