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
const Widget = 'widget';
const Action = 'action';
const Section = 'section';
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
const nickel = Object.freeze({
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
    notifications: Object.freeze({
        get() { return __nickelResource('notifications', {notification:null,history:[],historyVisible:false}); }
    }),
    tray: Object.freeze({
        list() { return __nickelResource('tray', []); },
        activate(id) { __effects.push({type:'tray.activate',id:__nickelIdentity(id)}); },
        contextMenu(id) { __effects.push({type:'tray.contextMenu',id:__nickelIdentity(id)}); }
    }),
    windows: Object.freeze({
        list() { return __nickelResource('windows', []); },
        activate(id) { __effects.push({type:'windows.focus',id:__nickelIdentity(id)}); },
        close(id) { __effects.push({type:'windows.close',id:__nickelIdentity(id)}); }
    }),
    applications: Object.freeze({
        list() { return __nickelResource('applications', []); },
        launch(id) { __effects.push({type:'applications.launch',id:__nickelIdentity(id)}); },
        togglePin(id) { __effects.push({type:'applications.togglePin',id:__nickelIdentity(id)}); },
        movePin(id, direction) {
            if (direction !== -1 && direction !== 1) throw TypeError('pin direction must be -1 or 1');
            __effects.push({type:'applications.movePin',id:__nickelIdentity(id),direction});
        }
    }),
    component(contract) {
        const component = __nickelPublicComponents.get(contract);
        if (!component) throw Error(`unknown public component ${contract}`);
        return component;
    },
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
        get() { return __nickelResource('wifi', {available:false, reason:'Wi-Fi read capability is unavailable', networks:[], operations:{}}); },
        listNetworks() { return this.get().networks; },
        setEnabled(value) { __nickelConnectivityEffect('wifi', 'setEnabled', value); },
        connect(id) { __nickelConnectivityEffect('wifi', 'connect', __nickelIdentity(id), true); }
    }),
    bluetooth: Object.freeze({
        get() { return __nickelResource('bluetooth', {available:false, reason:'Bluetooth read capability is unavailable', devices:[], operations:{}}); },
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
    displays: Object.freeze({
        get() {
            const snapshot = __nickelData.displays;
            return snapshot === undefined ? undefined : JSON.parse(JSON.stringify(snapshot));
        },
        setLayout(layout) {
            if (layout === null || typeof layout !== 'object' || Array.isArray(layout)
                || typeof layout.primary !== 'string'
                || !Array.isArray(layout.placements)
                || layout.placements.length < 1 || layout.placements.length > 32)
                throw TypeError('invalid display layout');
            __effects.push({type: 'displays.setLayout', layout: JSON.parse(JSON.stringify(layout))});
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
        value: props?.value, placeholder: props?.placeholder, secure: props?.secure,
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
