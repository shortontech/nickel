
const Panel = 'panel';
const Surface = 'surface';
const Box = 'box';
const FileTile = 'file-tile';
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
const TextField = 'text-field';
const Button = 'button';
const Spacer = 'spacer';
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
let __effects = [];
let __pendingRender = null;
let __pendingEvent = null;
let __nickelData = Object.freeze({query: '', results: []});

const nickel = Object.freeze({
    request(effect) { __effects.push(effect); },
    openDialog(id) { __effects.push(`open-dialog:${id}`); },
    openMenu(id) { __effects.push(`open-menu:${id}`); },
    get data() { return __nickelData; }
});

function __nickelSetData(data) { __nickelData = Object.freeze(data); }

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
            return node;
        } finally {
            __currentComponent = previous;
            __hookIndex = previousIndex;
        }
    }
    const handler = typeof props?.onClick === 'function' ? props.onClick : props?.onChange;
    const action = typeof handler === 'function' ? __handlers.push(handler) - 1 : null;
    const contextAction = typeof props?.onContextMenu === 'function'
        ? __handlers.push(props.onContextMenu) - 1 : null;
    const dragAction = typeof props?.onDrag === 'function'
        ? __handlers.push(props.onDrag) - 1 : null;
    const selectAction = typeof props?.onSelect === 'function'
        ? __handlers.push(props.onSelect) - 1 : null;
    const moveAction = typeof props?.onMove === 'function'
        ? __handlers.push(props.onMove) - 1 : null;
    const closeAction = typeof props?.onClose === 'function'
        ? __handlers.push(props.onClose) - 1 : null;
    return {kind, action, id: props?.id, open: props?.open, anchor: props?.anchor,
        x: props?.x, y: props?.y, width: props?.width, height: props?.height,
        background: props?.background, radius: props?.radius, color: props?.color,
        label: props?.label, selected: props?.selected, hovered: props?.hovered,
        dragging: props?.dragging, outline: props?.outline,
        hoverBackground: props?.hoverBackground,
        selectedBackground: props?.selectedBackground, accent: props?.accent,
        complement: props?.complement,
        item: props?.item, count: props?.count, hue: props?.hue, custom: props?.custom,
        asset: props?.asset, fit: props?.fit,
        accessibilityLabel: props?.accessibilityLabel, state: props?.state, icon: props?.icon,
        showLabel: props?.showLabel, contextAction, dragAction, selectAction, moveAction, closeAction,
        value: props?.value, placeholder: props?.placeholder,
        percent: props?.percent,
        children: children.flat(Infinity).filter(child => child !== null && child !== false)};
}

function __nickelRollbackRender() {
    if (__pendingRender !== null) {
        const {handlers, hooks, values, effectsLength} = __pendingRender;
        __handlers = handlers;
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
    const {handlers, hooks, values, effectsLength, effects} = __pendingEvent;
    __handlers = handlers;
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

function __nickelRender() {
    if (__pendingRender !== null) throw Error('previous render was not finalized');
    const previousHandlers = __handlers;
    const previousHooks = new Map(Array.from(__componentHooks, ([path, hooks]) => [path, hooks.slice()]));
    const previousValues = Array.from(__componentHooks.values(), hooks => hooks.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    __pendingRender = {handlers: previousHandlers, hooks: previousHooks,
        values: previousValues, effectsLength: __effects.length};
    __handlers = [];
    __visitedComponents = new Set();
    __componentChildren = new Map();
    __currentComponent = null;
    __hookIndex = 0;
    try {
        const node = h(App, {});
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
    const handler = __handlers[action];
    if (!handler) return __nickelRender();
    const hooks = new Map(Array.from(__componentHooks, ([path, slots]) => [path, slots.slice()]));
    const values = Array.from(__componentHooks.values(), slots => slots.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    const effectsLength = __effects.length;
    __pendingEvent = {handlers: __handlers, hooks, values,
        effectsLength, effects: __effects.slice()};
    try {
        handler(value);
        return __nickelRender();
    } catch (error) {
        __nickelRollbackEvent();
        throw error;
    }
}
