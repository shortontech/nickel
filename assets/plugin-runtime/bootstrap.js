const __nickelPublicComponents = new Map();
function __nickelPublishComponent(contract, component) {
    if (typeof component !== 'function') throw TypeError(`public component ${contract} is not a function`);
    __nickelPublicComponents.set(contract, component);
}

const Window = 'window';
const __fragmentChildren = new WeakSet();
function Fragment({children}) {
    __fragmentChildren.add(children);
    return children;
}
function FixedWindow(props) {
    const {children, ...windowProps} = props;
    return h(Window, {...windowProps, placement: 'fixed'}, ...children);
}
function Panel(props) {
    const {children, height, ...surfaceProps} = props || {};
    const panelHeight = Number.isInteger(height) && height >= 1 && height <= 8192 ? height : 64;
    const surface = nickel.data.surface;
    const background = surfaceProps.background ?? 0xc9262b36;
    const content = surface && surface.width && surface.height
        ? h(Box, {
            x: 0, y: Math.max(0, surface.height - panelHeight),
            width: surface.width, height: Math.min(panelHeight, surface.height),
            background, radius: 16
        }, h(Row, null, ...children))
        : h(Row, null, ...children);
    return h(FixedWindow,
        {width: '100%', ...surfaceProps, height: '100%',
            background: surface ? 0 : background},
        content);
}
const Box = 'box';
const Layer = 'layer';
const Div = 'div';
const Badge = 'badge';
const Row = 'row';
const Column = 'column';
const ScrollView = 'scroll-view';
const Text = 'text';
const Image = 'image';
const ImageButton = 'image-button';
const Progress = 'progress';
// Native slider input is normalized; JSX accepts ordinary numeric ranges.
function Slider(props) {
    const {min = 0, max = 1, step, value, onChange, children, ...rest} = props;
    if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min
        || !Number.isFinite(value) || value < min || value > max
        || (step !== undefined && (!Number.isFinite(step) || step <= 0)))
        throw RangeError('invalid slider range, value, or step');
    if (typeof onChange !== 'function') throw TypeError('slider requires onChange');
    return h('slider', {...rest, value:(value - min) / (max - min), onChange:fraction => {
        let next = min + Math.max(0, Math.min(1, fraction)) * (max - min);
        if (step !== undefined) next = min + Math.round((next - min) / step) * step;
        onChange(Math.max(min, Math.min(max, next)));
    }}, ...(children || []));
}
const Switch = 'switch';
function Checkbox(props) {
    const {checked = false, indeterminate = false, disabled = false, onChange, onClick, children, ...rest} = props;
    if (typeof checked !== 'boolean' || typeof indeterminate !== 'boolean' || typeof disabled !== 'boolean') throw TypeError('checkbox flags must be booleans');
    const state = disabled ? (indeterminate ? 'mixed-unavailable' : checked ? 'disabled-on' : 'disabled-off') : indeterminate ? 'mixed' : checked ? 'on' : 'off';
    const handler = disabled ? undefined : typeof onChange === 'function' ? () => onChange(!checked) : onClick;
    return h('checkbox', {...rest, accessibilityLabel:rest.accessibilityLabel ?? rest['aria-label'] ?? rest.label, state, onClick:handler}, ...(children || []));
}
const ColorSwatch = 'color-swatch';
const Select = 'select';
const Option = 'option';
const TextField = 'text-field';
const Button = 'button';
const Spacer = 'spacer';
const Slot = 'slot';
const Dialog = 'dialog';
const Menu = 'menu';
const MenuItem = 'menu-item';
const __componentIds = new WeakMap();
let __nextComponentId = 0;
let __componentHooks = new Map();
let __visitedComponents = new Set();
let __componentChildren = new Map();
let __currentComponent = null;
let __hookIndex = 0;
let __handlers = [];
let __previousHandlers = [];
let __effects = [];
let __listKeyErrors = [];
let __pendingRender = null;
let __pendingEvent = null;
let __nickelData = Object.freeze({query: '', results: []});
let __activeSurface = 'default';
const __surfaceStates = new Map();
const __surfaceApps = new Map();

function __nickelRegisterSurfaceApp(id, component) {
    if (typeof id !== 'string' || !id.length || typeof component !== 'function')
        throw Error('invalid surface entry');
    if (__surfaceApps.has(id)) throw Error('surface entry is already registered');
    __surfaceApps.set(id, component);
}

// Registrations retain executable values in this package's module graph. Only
// declarative metadata crosses the host boundary; package identity comes from Rust.
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
function readPluginSettings() { return __readSettings(__settingsSnapshot, false); }
function readSettingsPages() { return __readSettings(__settingsPagesSnapshot, true); }
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

function __nickelResource(name, fallback) {
    return JSON.parse(JSON.stringify(__nickelData[name] === undefined ? fallback : __nickelData[name]));
}
function __nickelIdentity(id) {
    if (typeof id !== 'string' || !id.length || id.length > 512) throw TypeError('invalid capability identity');
    return id;
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
        dismissMenu() { __effects.push({type:'windows.dismissMenu'}); },
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
            if (!snapshot.available || !snapshot.writable || !plugin || !plugin.shell) throw Error('shell selection is unavailable');
            if (typeof revision !== 'string' || revision !== snapshot.revision) throw Error('plugin inventory is stale');
            __effects.push({type:'plugins.selectShell',id,revision});
        },
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
        switch(id) { this.perform('switch',id); },
        create() { this.perform('create'); },
        remove(id) { this.perform('remove',id); },
        perform(operation,id) {
            const snapshot = this.get();
            if (!snapshot.available || !snapshot.operations[operation]) throw Error('workspace operation unavailable');
            const effect = {type:'workspaces.'+operation,revision:snapshot.revision};
            if (operation !== 'create') {id=__nickelIdentity(id); if (!snapshot.workspaces.some(workspace=>workspace.id===id)) throw Error('unknown workspace'); effect.id=id;}
            __effects.push(effect);
        }
    }),
    desktop: Object.freeze({
        get() { return __nickelResource('desktop',{available:false,operations:{}}); },
        toggleShowDesktop() { if (!this.get().operations.toggleShowDesktop) throw Error('show desktop unavailable'); __effects.push({type:'desktop.toggleShowDesktop'}); }
    }),
    displays: Object.freeze({
        get() {
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

function __nickelSetData(data) { __nickelData = Object.freeze(data); }

function __nickelSelectSurface(id) {
    if (typeof id !== 'string' || !id.length) throw Error('invalid surface identity');
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot switch surfaces during a render or event');
    if (id === __activeSurface) return;
    if (__activeSurface) __surfaceStates.set(__activeSurface, {
        hooks: __componentHooks,
        handlers: __handlers,
        previousHandlers: __previousHandlers,
        effects: __effects,
        data: __nickelData
    });
    const state = __surfaceStates.get(id);
    __componentHooks = state?.hooks ?? new Map();
    __handlers = state?.handlers ?? [];
    __previousHandlers = state?.previousHandlers ?? [];
    __effects = state?.effects ?? [];
    __nickelData = state?.data ?? Object.freeze({query: '', results: []});
    __visitedComponents = new Set();
    __componentChildren = new Map();
    __currentComponent = null;
    __hookIndex = 0;
    __listKeyErrors = [];
    __activeSurface = id;
}

function __nickelDropSurface(id) {
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot retire a surface during a render or event');
    __surfaceStates.delete(id);
    __surfaceApps.delete(id);
    if (id !== __activeSurface) return;
    __componentHooks = new Map();
    __handlers = [];
    __previousHandlers = [];
    __effects = [];
    __nickelData = Object.freeze({query: '', results: []});
    __visitedComponents = new Set();
    __componentChildren = new Map();
    __currentComponent = null;
    __hookIndex = 0;
    __listKeyErrors = [];
    __activeSurface = '';
}

function __nickelTakeEffects() {
    if (__compositionCheckpoint !== null) {
        if (__compositionCheckpoint.activeSurface === __activeSurface) __compositionCheckpoint.active.effects = [];
        const saved = __compositionCheckpoint.surfaces.get(__activeSurface);
        if (saved) saved.effects = [];
    }
    return JSON.stringify(__effects.splice(0));
}

function useState(initial) {
    if (__currentComponent === null) throw Error('useState requires a component');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    if (!hooks[slot]) hooks[slot] = {kind: 'state', value: typeof initial === 'function' ? initial() : initial};
    if (hooks[slot].kind !== 'state') throw Error('hook order changed');
    const entry = hooks[slot];
    const owner = __currentComponent;
    return [entry.value, next => {
        if (__componentHooks.get(owner)?.[slot] !== entry) return;
        entry.value = typeof next === 'function' ? next(entry.value) : next;
    }];
}

function useRef(initial) {
    if (__currentComponent === null) throw Error('useRef requires a component');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    if (!hooks[slot]) hooks[slot] = {kind: 'ref', value: {current: initial}};
    if (hooks[slot].kind !== 'ref') throw Error('hook order changed');
    return hooks[slot].value;
}

function h(kind, props, ...children) {
    if (typeof kind === 'function') {
        let type = __componentIds.get(kind);
        if (type === undefined) {
            type = ++__nextComponentId;
            __componentIds.set(kind, type);
        }
        const parent = __currentComponent ?? 'root';
        const ordinalKey = `${parent}/${type}`;
        const ordinal = __componentChildren.get(ordinalKey) ?? 0;
        __componentChildren.set(ordinalKey, ordinal + 1);
        const identity = props?.key === undefined ? `#${ordinal}` : `@${encodeURIComponent(String(props.key))}`;
        const path = `${ordinalKey}/${identity}`;
        if (__visitedComponents.has(path)) throw Error(`duplicate component key ${identity}`);
        __visitedComponents.add(path);
        if (!__componentHooks.has(path)) __componentHooks.set(path, []);
        const previous = __currentComponent;
        const previousIndex = __hookIndex;
        __currentComponent = path;
        __hookIndex = 0;
        try {
            const node = kind({...props, children});
            if (__hookIndex !== __componentHooks.get(path).length) throw Error('hook order changed');
            return props?.key === undefined || node === null || typeof node !== 'object' || Array.isArray(node)
                ? node : {...node, key: props.key};
        } finally {
            __currentComponent = previous;
            __hookIndex = previousIndex;
        }
    }
    for (const child of children) {
        if (!Array.isArray(child) || __fragmentChildren.has(child)) continue;
        const seen = new Set();
        for (const item of child.flat(Infinity).filter(item => item !== null && item !== false)) {
            if (typeof item !== 'object') continue;
            if (item?.key === undefined) {
                __listKeyErrors.push('items rendered from an array need a stable key');
                continue;
            }
            const key = String(item.key);
            if (seen.has(key)) __listKeyErrors.push(`duplicate list key ${key}`);
            seen.add(key);
        }
    }
    const handler = typeof props?.onClick === 'function' ? props.onClick : props?.onChange;
    const action = typeof handler === 'function' ? __handlers.push(handler) - 1 : null;
    const contextAction = typeof props?.onContextMenu === 'function'
        ? __handlers.push(props.onContextMenu) - 1 : null;
    const dragAction = typeof props?.onDrag === 'function'
        ? __handlers.push(props.onDrag) - 1 : null;
    const dropAction = typeof props?.onDrop === 'function'
        ? __handlers.push(props.onDrop) - 1 : null;
    const focusAction = typeof props?.onFocus === 'function'
        ? __handlers.push(props.onFocus) - 1 : null;
    const blurAction = typeof props?.onBlur === 'function'
        ? __handlers.push(props.onBlur) - 1 : null;
    const selectAction = typeof props?.onSelect === 'function'
        ? __handlers.push(props.onSelect) - 1 : null;
    const moveAction = typeof props?.onMove === 'function'
        ? __handlers.push(props.onMove) - 1 : null;
    const fileAction = typeof props?.onFileAction === 'function'
        ? __handlers.push(props.onFileAction) - 1 : null;
    const closeAction = typeof props?.onClose === 'function'
        ? __handlers.push(props.onClose) - 1 : null;
    const escapeAction = typeof props?.onEscape === 'function'
        ? __handlers.push(props.onEscape) - 1 : null;
    const submitAction = typeof props?.onSubmit === 'function'
        ? __handlers.push(props.onSubmit) - 1 : null;
    return {kind, key: props?.key, action, id: props?.id, title: props?.title, className: props?.className, open: props?.open, anchor: props?.anchor,
        placement: props?.placement, output: props?.output, edge: props?.edge,
        reserveWorkArea: props?.reserveWorkArea, bottomOffset: props?.bottomOffset,
        x: props?.x, y: props?.y, width: props?.width, height: props?.height, grow: props?.grow,
        background: props?.background, padding: props?.padding, radius: props?.radius, color: props?.color,
        label: props?.label, disabledReason: props?.disabledReason,
        shortcut: props?.shortcut, separatorBefore: props?.separatorBefore,
        selected: props?.selected, hovered: props?.hovered,
        dragging: props?.dragging, outline: props?.outline,
        hoverBackground: props?.hoverBackground,
        selectedBackground: props?.selectedBackground, accent: props?.accent,
        complement: props?.complement,
        item: props?.item, count: props?.count, hue: props?.hue, custom: props?.custom,
        asset: props?.asset, fit: props?.fit,
        accessibilityLabel: props?.accessibilityLabel, role: props?.role,
        'aria-label': props?.['aria-label'], 'aria-checked': props?.['aria-checked'],
        'aria-selected': props?.['aria-selected'],
        state: props?.state, disabled: props?.disabled, icon: props?.icon,
        showLabel: props?.showLabel, contextAction, dragAction, dropAction, focusAction, blurAction,
        selectAction, moveAction, fileAction, closeAction,
        escapeAction, submitAction,
        value: props?.value, placeholder: props?.placeholder, secure: props?.secure, autoFocus: props?.autoFocus,
        wrap: props?.wrap,
        maxLines: props?.maxLines,
        percent: props?.percent,
        children: children.flat(Infinity).filter(child => child !== null && child !== false)};
}

function __nickelRollbackRender() {
    if (__pendingRender !== null) {
        const {handlers, previousHandlers, hooks, values, effectsLength} = __pendingRender;
        __handlers = handlers;
        __previousHandlers = previousHandlers;
        __nickelRestoreHooks(hooks, values, effectsLength);
        __pendingRender = null;
    }
    __nickelRollbackEvent();
}

function __nickelRestoreHooks(hooks, values, effectsLength) {
    let componentIndex = 0;
    for (const slots of hooks.values()) {
        const oldValues = values[componentIndex++];
        slots.forEach((entry, index) => {
            if (entry.kind === 'ref') entry.value.current = oldValues[index];
            else entry.value = oldValues[index];
        });
    }
    __componentHooks = hooks;
    __effects.length = effectsLength;
}

function __nickelRollbackEvent() {
    if (__pendingEvent === null) return;
    const {handlers, previousHandlers, hooks, values, effectsLength, effects} = __pendingEvent;
    __handlers = handlers;
    __previousHandlers = previousHandlers;
    __nickelRestoreHooks(hooks, values, effectsLength);
    __effects = effects;
    __pendingEvent = null;
}

function __nickelCommitRender() {
    __pendingRender = null;
}

function __nickelAcceptEvent() {
    __pendingEvent = null;
}

function __nickelActiveEntry() {
    return __surfaceApps.get(__activeSurface) || App;
}

function __nickelRender(component = __nickelActiveEntry()) {
    if (__pendingRender !== null) throw Error('previous render was not finalized');
    const previousHandlers = __handlers;
    const olderHandlers = __previousHandlers;
    const previousHooks = new Map(Array.from(__componentHooks, ([path, hooks]) => [path, hooks.slice()]));
    const previousValues = Array.from(__componentHooks.values(), hooks => hooks.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    __pendingRender = {handlers: previousHandlers, previousHandlers: olderHandlers, hooks: previousHooks,
        values: previousValues, effectsLength: __effects.length};
    __handlers = [];
    __previousHandlers = previousHandlers;
    __listKeyErrors = [];
    __visitedComponents = new Set();
    __componentChildren = new Map();
    __currentComponent = null;
    __hookIndex = 0;
    try {
        const node = h(component, {});
        if (node?.kind === 'window' && __listKeyErrors.length) throw Error(__listKeyErrors[0]);
        for (const path of __componentHooks.keys()) {
            if (!__visitedComponents.has(path)) __componentHooks.delete(path);
        }
        return JSON.stringify(node);
    } catch (error) {
        __nickelRollbackRender();
        throw error;
    }
}

function __nickelDispatch(action, value) {
    return __nickelDispatchBatch([[action, value]]);
}

function __nickelDispatchRemovedFocus(action) {
    return __nickelDispatchBatch([[action]], true);
}

function __nickelDispatchBatch(events, previous = false) {
    if (!events.length) return __nickelRender();
    const hooks = new Map(Array.from(__componentHooks, ([path, slots]) => [path, slots.slice()]));
    const values = Array.from(__componentHooks.values(), slots => slots.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    const effectsLength = __effects.length;
    __pendingEvent = {handlers: __handlers, previousHandlers: __previousHandlers, hooks, values,
        effectsLength, effects: __effects.slice()};
    try {
        for (const [action, value] of events) {
            const handler = (previous ? __previousHandlers : __handlers)[action];
            if (handler) handler(value);
        }
        return __nickelRender();
    } catch (error) {
        __nickelRollbackEvent();
        throw error;
    }
}

// Native composition checkpoints cover bootstrap-owned presentation state.
// They deliberately do not snapshot package globals or closure-captured values.
let __compositionCheckpoint = null;
function __nickelBeginCheckpoint() {
    if (__compositionCheckpoint !== null || __pendingRender !== null || __pendingEvent !== null) throw Error('checkpoint already pending');
    const seen = new Map();
    const extensible = new Map();
    let nodes = 0;
    function copy(value, depth = 0) {
        if (value === null || typeof value !== 'object') return value;
        if (depth > 128 || ++nodes > 16384) throw Error('checkpoint value exceeds limit');
        if (seen.has(value)) return seen.get(value);
        if (!Array.isArray(value) && Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) throw Error('unsupported checkpoint hook value');
        const result = Array.isArray(value) ? [] : Object.create(Object.getPrototypeOf(value));
        seen.set(value, result);
        extensible.set(value, Object.isExtensible(value));
        for (const key of Reflect.ownKeys(value)) {
            const descriptor = Object.getOwnPropertyDescriptor(value, key);
            if (!descriptor || !('value' in descriptor)) throw Error('checkpoint cannot invoke hook accessors');
            Object.defineProperty(result, key, {...descriptor, value:copy(descriptor.value, depth + 1)});
        }
        return result;
    }
    function state(hooks, handlers, previousHandlers, effects, data) {
        return {hooks:new Map(Array.from(hooks, ([path, slots]) => [path, slots.slice()])),
            values:Array.from(hooks.values(), slots => slots.map(entry => copy(entry.kind === 'ref' ? entry.value.current : entry.value))),
            handlers:handlers.slice(), previousHandlers:previousHandlers.slice(), effects:copy(effects), data};
    }
    const active = state(__componentHooks, __handlers, __previousHandlers, __effects, __nickelData);
    const surfaces = new Map(Array.from(__surfaceStates, ([id, value]) => [id, state(value.hooks, value.handlers, value.previousHandlers, value.effects, value.data)]));
    __compositionCheckpoint = {active, surfaces, graph:seen, extensible, apps:new Map(__surfaceApps), activeSurface:__activeSurface,
        settingsValues:copy(__settingsValues), settingsSnapshot:copy(__settingsSnapshot), settingsPagesSnapshot:copy(__settingsPagesSnapshot)};
}
function __nickelFinishCheckpoint(accepted) {
    const checkpoint = __compositionCheckpoint;
    if (checkpoint === null) throw Error('checkpoint is unavailable');
    if (__pendingRender !== null || __pendingEvent !== null) { if (accepted) throw Error('unfinished local checkpoint transaction'); __nickelRollbackRender(); }
    __compositionCheckpoint = null;
    if (accepted) return;
    // Restore supported mutable hook objects in place: existing approved event
    // closures can retain these identities. Arbitrary closure cells/globals are
    // still outside this graph and are not presented as rolled back.
    const originals = new Map(Array.from(checkpoint.graph, ([original, snapshot]) => [snapshot, original]));
    const original = value => originals.has(value) ? originals.get(value) : value;
    for (const [target, snapshot] of checkpoint.graph) {
        if (Object.isExtensible(target) !== checkpoint.extensible.get(target)) throw Error('checkpoint hook integrity cannot be restored');
        if (Object.getPrototypeOf(target) !== Object.getPrototypeOf(snapshot) && !Reflect.setPrototypeOf(target, Object.getPrototypeOf(snapshot))) throw Error('checkpoint hook prototype cannot be restored');
        for (const key of Reflect.ownKeys(target)) { if (!Object.prototype.hasOwnProperty.call(snapshot, key) && !Reflect.deleteProperty(target, key)) throw Error('checkpoint hook property cannot be restored'); }
        for (const key of Reflect.ownKeys(snapshot)) {
            const descriptor = Object.getOwnPropertyDescriptor(snapshot, key);
            Object.defineProperty(target, key, {...descriptor, value:original(descriptor.value)});
        }
    }
    function restore(state) {
        let index = 0;
        for (const slots of state.hooks.values()) {
            const values = state.values[index++];
            slots.forEach((entry, slot) => { if (entry.kind === 'ref') entry.value.current = original(values[slot]); else entry.value = original(values[slot]); });
        }
        return {hooks:state.hooks, handlers:state.handlers, previousHandlers:state.previousHandlers, effects:original(state.effects), data:state.data};
    }
    __surfaceStates.clear();
    for (const [id, state] of checkpoint.surfaces) __surfaceStates.set(id, restore(state));
    __surfaceApps.clear();
    for (const [id, app] of checkpoint.apps) __surfaceApps.set(id, app);
    const active = restore(checkpoint.active);
    __componentHooks = active.hooks; __handlers = active.handlers; __previousHandlers = active.previousHandlers;
    __effects = active.effects; __nickelData = active.data; __activeSurface = checkpoint.activeSurface;
    __settingsValues = original(checkpoint.settingsValues); __settingsSnapshot = original(checkpoint.settingsSnapshot); __settingsPagesSnapshot = original(checkpoint.settingsPagesSnapshot);
    __visitedComponents = new Set(); __componentChildren = new Map(); __currentComponent = null; __hookIndex = 0; __listKeyErrors = [];
}
