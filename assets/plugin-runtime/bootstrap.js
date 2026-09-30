
const Window = 'window';
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
const Slider = 'slider';
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

const nickel = Object.freeze({
    request(effect) { __effects.push(effect); },
    openDialog(id) { __effects.push(`open-dialog:${id}`); },
    openMenu(id) { __effects.push(`open-menu:${id}`); },
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
        if (!Array.isArray(child)) continue;
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
        showLabel: props?.showLabel, contextAction, dragAction, focusAction, blurAction,
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
