const __nickelPublicComponents = new Map();
// The bounded ambient API intentionally available to package modules. Keep
// this list synchronized with nickel-plugin.d.ts; __nickel internals are not
// public merely because package code shares this lexical runtime context.
let __nickelPublicRuntimeGlobals = Object.freeze([
    'Window','Fragment','FixedWindow','Panel','Box','Layer','Div','Badge','Row','Column',
    'ScrollView','VirtualColumn','Text','Image','ImageButton','Progress','Slider','Switch','Checkbox',
    'ColorSwatch','Select','Option','TextField','Button','Spacer','Slot','Dialog','Menu',
    'MenuItem','ErrorBoundary','TwinkleStores',
    'twinkle',

    'useLocale','useSyncExternalStore','memo','useTheme','useReducedMotion',
    'useHostCapability','useSurface','useOutput','useScaleFactor','useSurfaceFocus','useState',
    'useReducer','useRef','useId','createContext','useContext','useMemo','useCallback',
    'useEffect','h'
]);
const __nickelComponentMetadata = new WeakMap();
let __nickelHotSignatures = null;
let __nickelHotPrevious = null;
let __nickelPackageOwner = null;
function __nickelSetDiagnosticOwner(owner) { __nickelPackageOwner = owner; }
function __nickelPublishComponent(contract, component, module = null, exported = null) {
    if (typeof component !== 'function') throw TypeError(`public component ${contract} is not a function`);
    const previous = __nickelHotPrevious?.get(contract);
    const key = `${module}#${exported ?? contract}`;
    if (previous && __nickelHotSignatures?.[key] !== undefined) {
        const metadata = __nickelComponentMetadata.get(previous);
        const previousKey = `${metadata?.module}#${metadata?.export}`;
        if (previousKey === key && __nickelHotSignatures[key] !== null
            && __componentIds.has(previous))
            __componentIds.set(component, __componentIds.get(previous));
    }
    __nickelPublicComponents.set(contract, component);
    __nickelComponentMetadata.set(component, Object.freeze({module,export:exported ?? contract}));
}

function __nickelInstallHotModules(installer, signatures) {
    const previous = new Map(__nickelPublicComponents);
    __nickelHotPrevious = previous; __nickelHotSignatures = signatures;
    try { installer(); }
    catch (error) {
        __nickelPublicComponents.clear();
        for (const [key,value] of previous) __nickelPublicComponents.set(key,value);
        throw error;
    } finally { __nickelHotPrevious = null; __nickelHotSignatures = null; }
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
    const surface = __nickelData.surface;
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
// Key the component scope as well as its native wrapper. A native-only key
// does not isolate hook slots in renderItem's component descendants: changing
// the admitted slice would otherwise transfer nested window state to a sibling.
function __NickelVirtualRow({children}) {
    return h('column', null, ...children);
}
function VirtualColumn(props) {
    const source = useMemo(() => {
        if (!Array.isArray(props.items) || props.items.length > 10000)
            throw new Error('VirtualColumn needs at most 10000 items');
        if (typeof props.itemKey !== 'function' || typeof props.renderItem !== 'function')
            throw new Error('VirtualColumn needs itemKey and renderItem');
        const keys = props.items.map((item, index) => String(props.itemKey(item, index)));
        if (keys.some(key => !key || key.length > 512) || new Set(keys).size !== keys.length)
            throw new Error('VirtualColumn needs unique bounded item keys');
        const heights = props.items.map((item, index) => typeof props.itemHeight === 'function'
            ? props.itemHeight(item, index) : props.itemHeight);
        if (heights.some(height => !Number.isFinite(height) || height < 1 || height > 8192))
            throw new Error('VirtualColumn needs finite positive item heights');
        const uniform = heights.length === 0 || heights.every(height => height === heights[0]);
        return {keys, geometry:uniform ? {keys, count:heights.length, height:heights[0] ?? 1} : {keys, heights}};
    }, [props.items, props.itemKey, props.itemHeight, props.gap]);
    const [window, setWindow] = useState({source:null, start:0, end:0, reference:null});
    // Keep the previous native-selected slice provisionally during source
    // replacement. Clearing it would retire surviving row focus/capture before
    // native feedback can acknowledge the new source. Cold mounts stay empty.
    const current = window.source === source ? window : {
        start:Math.min(window.start, source.keys.length),
        end:Math.min(window.end, source.keys.length)
    };
    const onWindow = value => {
        const next = JSON.parse(value);
        if (!Number.isInteger(next.start) || !Number.isInteger(next.end)
            || next.start < 0 || next.start > next.end || next.end > source.keys.length)
            throw new Error('Invalid native virtual window');
        if (!Number.isSafeInteger(next.source) || next.source <= 0)
            throw new Error('Invalid native virtual source acknowledgement');
        const reference = next.source;
        setWindow(previous => previous.source === source && previous.start === next.start && previous.end === next.end && previous.reference === reference
            ? previous : {source, start:next.start, end:next.end, reference});
    };
    const rows = [];
    for (let index = current.start; index < current.end; index++)
        rows.push(h(__NickelVirtualRow, {key:source.keys[index]}, props.renderItem(props.items[index], index)));
    return h('div', {id:props.id, className:props.className, onChange:onWindow,
        collection:{...(current.reference ? {source:current.reference} : source.geometry), gap:props.gap ?? 0, overscan:props.overscan ?? 96,
            start:current.start, end:current.end}}, rows);
}
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
let __componentRecords = new Map();
// Legacy capability clients still take part in retained invalidation. Track
// their top-level snapshot reads per component so unrelated data stays cold.
let __componentResourceReads = new Map();
let __visitedComponents = new Set();
let __componentChildren = new Map();
// Render-local adjacency for the previously admitted component tree. Reusing
// a leaf must not scan every retained record merely to prove that the leaf's
// (possibly empty) descendant set is still live.
let __retainedComponentChildren = new Map();
let __currentComponent = null;
let __componentExecutionStack = [];
let __hookIndex = 0;
class __NickelHandlers extends Map {
    // Retiring a row releases its closure without recycling an issued action.
    // Transaction copies scale with live entries, not the highest issued ID.
    constructor(previous) { super(previous); this.nextAction=previous?.nextAction??0; }
    get length() { return this.size; }
    slice() { return new __NickelHandlers(this); }
    allocate() {
        if(this.nextAction>=Number.MAX_SAFE_INTEGER)throw Error('component handler identity exhausted');
        return this.nextAction++;
    }
}
let __handlers = new __NickelHandlers();
let __previousHandlers = new __NickelHandlers();
let __handlerSlots = new Map();
const __handlerBindings = new WeakSet();
const __virtualNativeNodes = new WeakSet();
const __resolvedVirtualValues = new WeakSet();
const __nonRetainedComponents = new WeakSet();
const __memoComponents = new WeakMap();
const __errorBoundaryComponents = new WeakSet();
const __MAX_BOUNDARY_DIAGNOSTICS = 32;
let __boundaryDiagnostics = [];
const __MAX_RUNTIME_REASONS = 32;
let __runtimeReasons = [];
const __MAX_RUNTIME_CHANGES = 32;
let __runtimeStoreChanges = [];
let __runtimeNativeMutations = [];
let __developerDiagnostics = [];
const __mountProfiles = new Map();
let __runtimeCounters = {renders:0,executed:0,reused:0,nativeNodesMaterialized:0,
    effectsScheduled:0,effectsRun:0,cleanups:0,failures:0,storeChanges:0,nativeMutations:0,
    mountBaseUploads:0,mountBaseReuses:0};
let __incrementalRender = false;
let __patchOnlyRender = false;
let __effects = [];
let __listKeyErrors = [];
let __pendingRender = null;
let __pendingEvent = null;
let __dirtyComponents = new Set();
let __acceptedNativeTree = null;
const __contexts = new WeakSet();
const __contextProviders = new WeakMap();
const __contextValues = new Map();
let __surfaceStore = {generation:0, snapshot:Object.freeze({
    generation:0, mountId:null, id:null, kind:null, logicalSize:null,
    output:null, availableSize:null, scaleFactor:null, focused:null, visible:null
})};

function ErrorBoundary() { throw Error('ErrorBoundary can only be rendered as a component'); }
__errorBoundaryComponents.add(ErrorBoundary);
function __nickelErrorMessage(error) {
    try { return String(error?.message ?? error).slice(0, 512); }
    catch (_) { return 'unprintable component failure'; }
}
function __nickelRecordBoundaryFailure(boundary, phase, error, surface = __activeSurface, state = null) {
    const message = __nickelErrorMessage(error);
    const previous = __boundaryDiagnostics[__boundaryDiagnostics.length - 1];
    if (previous && previous.surface === surface && previous.boundary === boundary
        && previous.phase === phase && previous.message === message) {
        previous.occurrences = Math.min(65535, previous.occurrences + 1); return;
    }
    if (__boundaryDiagnostics.length === __MAX_BOUNDARY_DIAGNOSTICS) __boundaryDiagnostics.shift();
    const stack = [];
    for (const [path, record] of state?.records ?? __componentRecords) {
        if (boundary !== path && !boundary.startsWith(`${path}/`)) continue;
        const metadata = __nickelComponentMetadata.get(record.kind);
        stack.push({path,module:metadata?.module ?? null,export:metadata?.export ?? record.kind?.name ?? null});
    }
    if (error?.__nickelComponent && !stack.some(entry => entry.path === error.__nickelComponent.path))
        stack.push(error.__nickelComponent);
    __runtimeCounters.failures++;
    __boundaryDiagnostics.push({package:__nickelPackageOwner,surface,mount:(state?.surfaceStore ?? __surfaceStore).snapshot.mountId,
        boundary,phase,message,hookIndex:__hookIndex,stack,occurrences:1});
}
function __nickelNearestBoundary(owner, records = __componentRecords) {
    if (typeof owner !== 'string') return null;
    let candidate = null;
    for (const [path, record] of records) {
        if (!record.boundary || (owner !== path && !owner.startsWith(`${path}/`))) continue;
        if (candidate === null || path.length > candidate.length) candidate = path;
    }
    return candidate;
}
function __nickelCaptureFailure(owner, error, phase, surface = __activeSurface) {
    const state = surface === __activeSurface
        ? {records:__componentRecords, dirty:__dirtyComponents, surfaceStore:__surfaceStore}
        : __surfaceStates.get(surface);
    if (!state) return false;
    const boundary = __nickelNearestBoundary(owner, state.records);
    if (boundary === null) return false;
    const record = state.records.get(boundary);
    state.records.set(boundary, {...record, boundaryError:error});
    state.dirty.add(boundary);
    __nickelRecordBoundaryFailure(boundary, phase, error, surface, state);
    return true;
}
// Native validation happens after JavaScript has produced a candidate patch.
// The host first rolls that candidate back, then reports the component owners
// whose admitted native boundaries were involved.  Capturing after rollback is
// important: the fallback becomes a fresh transaction based on the last
// committed tree and cannot inherit provisional handlers, hooks, or effects.
function __nickelCaptureNativeFailure(owners, message) {
    if (__pendingRender !== null || __pendingEvent !== null || __compositionCheckpoint !== null)
        throw Error('cannot capture native failure during a pending transaction');
    if (!Array.isArray(owners) || owners.length > 256)
        throw TypeError('native failure owners must be a bounded array');
    const error = Error(String(message).slice(0, 512));
    const captured = new Set();
    for (const owner of owners) {
        const boundary = __nickelNearestBoundary(owner);
        if (boundary === null || captured.has(boundary)) continue;
        const record = __componentRecords.get(boundary);
        // A fallback rejected by native validation must not spin forever.
        if (record?.boundaryError !== undefined) continue;
        __componentRecords.set(boundary, {...record, boundaryError:error});
        __dirtyComponents.add(boundary);
        __nickelRecordBoundaryFailure(boundary, 'native-validation', error);
        if(/invalid|unsupported|must|needs/i.test(error.message))
            __nickelDeveloperDiagnostic('invalid-props',error.message,
                'Check the native component prop contract and bounded values.',boundary);
        captured.add(boundary);
    }
    return JSON.stringify({captured:Array.from(captured)});
}
function __nickelBoundaryDiagnostics() { return JSON.stringify(__boundaryDiagnostics); }
function __nickelBoundedDiagnosticPush(target, value) {
    if (target.length === __MAX_RUNTIME_CHANGES) target.shift();
    target.push(value);
}
function __nickelDirtyComponentCount() {
    let count=__dirtyComponents.size;
    for (const state of __surfaceStates.values()) if (state.dirty!==__dirtyComponents) count+=state.dirty.size;
    return count;
}
function __nickelRecordStoreChange(store,generation,before) {
    const newlyDirty=Math.max(0,__nickelDirtyComponentCount()-before);
    __runtimeCounters.storeChanges++;
    __nickelBoundedDiagnosticPush(__runtimeStoreChanges,{store,generation,newlyDirty});
}
function __nickelRecordNativeMutations(operations,nodesVisited) {
    __runtimeCounters.nativeMutations+=operations.length;
    const kinds={}; for(const operation of operations) kinds[operation.op]=(kinds[operation.op]??0)+1;
    __nickelBoundedDiagnosticPush(__runtimeNativeMutations,{count:operations.length,nodesVisited,kinds});
}
function __nickelComponentStack(path) {
    if(__componentExecutionStack.some(entry=>entry.path===path))return __componentExecutionStack.slice(-16)
        .map(({path,module,export:exported})=>({path,module,export:exported}));
    const stack=[];
    for(const [candidate,record] of __componentRecords) {
        if(candidate!==path&&!path.startsWith(`${candidate}/`))continue;
        const metadata=__nickelComponentMetadata.get(record.kind);
        stack.push({path:candidate,module:metadata?.module??null,export:metadata?.export??record.kind?.name??null});
    }
    return stack.slice(-16);
}
function __nickelDeveloperDiagnostic(kind,message,suggestion,path=__currentComponent,detail={}) {
    const bounded=String(message).slice(0,512), owner=typeof path==='string'?path:null;
    const previous=__developerDiagnostics[__developerDiagnostics.length-1];
    if(previous&&previous.kind===kind&&previous.surface===__activeSurface&&previous.path===owner&&previous.message===bounded) {
        previous.occurrences=Math.min(65535,previous.occurrences+1);return;
    }
    if(__developerDiagnostics.length===__MAX_RUNTIME_CHANGES)__developerDiagnostics.shift();
    __developerDiagnostics.push({kind,severity:'warning',package:__nickelPackageOwner,surface:__activeSurface,
        mount:__surfaceStore.snapshot.mountId,path:owner,message:bounded,suggestion,stack:owner?__nickelComponentStack(owner):[],
        occurrences:1,...detail});
}
function __nickelProfile() {
    let profile=__mountProfiles.get(__activeSurface);
    if(!profile) {
        if(__mountProfiles.size===64) {
            const retired=Array.from(__mountProfiles.keys()).find(surface=>surface!==__activeSurface);
            if(retired!==undefined)__mountProfiles.delete(retired);
        }
        profile={surface:__activeSurface,mount:__surfaceStore.snapshot.mountId,renders:0,componentExecutions:0,
            componentExecutionMillis:0,reconciliationMillis:0,patchGenerationMillis:0,
            patches:0,patchOperations:0,patchNodesVisited:0,
            lifecycleCommits:0,lifecycleCommitMillis:0,timingPrecision:'wall-clock-milliseconds',
            hostTimingPrecision:'wall-clock-microseconds',
            nativeValidationMicros:0,coldTreeTransportBytes:0,patchEnvelopeTransportBytes:0,
            typedPatchApplyAttempts:0,typedPatchApplyRejections:0,typedPatchApplyMicros:0,
            consecutiveEffectTurns:0};
        __mountProfiles.set(__activeSurface,profile);
    }
    profile.mount=__surfaceStore.snapshot.mountId;return profile;
}
function __nickelReportTypedPatchApply(micros,accepted) {
    const profile=__nickelProfile();
    if(!Number.isSafeInteger(micros)||micros<0)return;
    profile.typedPatchApplyAttempts++;
    if(!accepted)profile.typedPatchApplyRejections++;
    profile.typedPatchApplyMicros+=micros;
}
function __nickelReportHostProfile(transportKind,nativeValidationMicros,transportBytes) {
    const profile=__nickelProfile();
    if(Number.isSafeInteger(nativeValidationMicros)&&nativeValidationMicros>=0)
        profile.nativeValidationMicros+=nativeValidationMicros;
    if(Number.isSafeInteger(transportBytes)&&transportBytes>=0) {
        if(transportKind==='cold-tree')profile.coldTreeTransportBytes+=transportBytes;
        else if(transportKind==='typed-patch')profile.patchEnvelopeTransportBytes+=transportBytes;
    }
}
function __nickelSelectedValue(entry,selector,snapshot,generation,store) {
    const selected=selector?selector(snapshot):snapshot;
    if(selector&&entry.generation===generation&&entry.value!==undefined&&!Object.is(selected,entry.value))
        __nickelDeveloperDiagnostic('unstable-selector',`${store} selector changed for an unchanged generation`,
            'Return a stable snapshot member or memoize the derived selector result.');
    return selected;
}
function __nickelSelectedStoreEntry(hooks,slot,kind,selector,store) {
    let entry=hooks[slot];
    if(entry?.kind===kind&&entry.selector!==selector)
        __nickelDeveloperDiagnostic('unstable-selector',`${store} selector function identity changed`,
            'Declare the selector outside the component or retain it with useCallback.');
    if(!entry||(entry.kind===kind&&entry.selector!==selector))
        hooks[slot]=entry={kind,selector,value:undefined,generation:0};
    if(entry.kind!==kind)throw Error('hook order changed');
    return entry;
}
function __nickelRetainedCounts() {
    // Counts only, never component data or closures. The selected surface's
    // saved entry can be stale/aliased; count its current state exactly once.
    const counts={surfaces:0,hookComponents:0,hookSlots:0,componentRecords:0,
        handlerEntries:0,previousHandlerEntries:0,handlerSlots:0};
    function add(hooks,records,handlers,previousHandlers,slots) {
        counts.surfaces++;counts.hookComponents+=hooks.size;
        for(const entries of hooks.values())counts.hookSlots+=entries.length;
        counts.componentRecords+=records.size;counts.handlerEntries+=handlers.size;
        counts.previousHandlerEntries+=previousHandlers.size;counts.handlerSlots+=slots.size;
    }
    if(__activeSurface)add(__componentHooks,__componentRecords,__handlers,__previousHandlers,__handlerSlots);
    for(const [id,state] of __surfaceStates)if(id!==__activeSurface)
        add(state.hooks,state.records,state.handlers,state.previousHandlers,state.handlerSlots);
    return counts;
}
function __nickelRuntimeDiagnostics() { return JSON.stringify({counters:__runtimeCounters,reasons:__runtimeReasons,
    retained:__nickelRetainedCounts(),
    storeChanges:__runtimeStoreChanges,nativeMutations:__runtimeNativeMutations,
    developerDiagnostics:__developerDiagnostics,profiles:Array.from(__mountProfiles.values())}); }
let __localeStore = {generation:0,snapshot:Object.freeze({generation:0,tag:'und',direction:'ltr',known:false})};
let __themeStore = {generation:0, snapshot:Object.freeze({
    generation:0, mode:'unknown', accent:null, accentHue:null, accentIntensity:null,
    reducedMotion:null, reducedTransparency:null, palette:null
})};
let __capabilityStore = {generation:0, known:Object.freeze([]), snapshot:Object.freeze({})};
let __nickelData = Object.freeze({query: '', results: []});
const __nickelExternalSubscriptions = new WeakMap();
const __nickelExternalSnapshots = new WeakMap();
function __nickelStoreContract(name, read) {
    const subscribe=()=>{throw Error('Nickel store subscriptions are installed only by useSyncExternalStore');};
    const getSnapshot=()=>read().snapshot;
    const descriptor=Object.freeze({name,read});
    __nickelExternalSubscriptions.set(subscribe,descriptor);__nickelExternalSnapshots.set(getSnapshot,descriptor);
    return Object.freeze({subscribe,getSnapshot});
}
const TwinkleStores=Object.freeze({
    locale:__nickelStoreContract('locale',()=>__localeStore),
    theme:__nickelStoreContract('theme',()=>__themeStore),
    capabilities:__nickelStoreContract('capabilities',()=>__capabilityStore)
});
function __nickelNotifyExternalStore(name) {
    __nickelForEachSurfaceHooks((hooks,dirty)=>{for(const [owner,slots] of hooks)for(const entry of slots){
        if(entry?.kind!=='sync-external-store'||entry.store.name!==name)continue;
        try{const state=entry.store.read(),snapshot=entry.getSnapshot();entry.storeError=undefined;
            if(state.generation!==entry.generation||!Object.is(snapshot,entry.value))dirty.add(owner);
        }catch(error){entry.storeError=error;dirty.add(owner);}
    }});
}
let __activeSurface = 'default';
const __surfaceStates = new Map();
const __surfaceApps = new Map();
const __surfaceAppIdentities = new Map();

function __nickelRegisterSurfaceApp(id, component, identity = null) {
    if (typeof id !== 'string' || !id.length || typeof component !== 'function')
        throw Error('invalid surface entry');
    if (__surfaceApps.has(id)) throw Error('surface entry is already registered');
    __surfaceApps.set(id, component);
    __surfaceAppIdentities.set(id, {...(identity ?? {owner:null,module:null,export:null,signature:null}),component});
}

// Native composition mounts select already-loaded components as data. Keep
// lookup inside App so provider replacement and page retirement remain live.
function __nickelReplaceSurfaceApp(id, component, identity) {
    if (typeof id !== 'string' || !__surfaceApps.has(id) || typeof component !== 'function')
        throw Error('hot replacement target is unavailable');
    if (!identity || typeof identity !== 'object' || Array.isArray(identity))
        throw Error('hot replacement identity is required');
    const previous = __surfaceAppIdentities.get(id);
    const compatible = previous.owner === identity.owner
        && previous.module === identity.module && previous.export === identity.export
        && previous.signature === identity.signature;
    if (compatible && __componentIds.has(previous.component))
        __componentIds.set(component, __componentIds.get(previous.component));
    if (!compatible) {
        const state = id === __activeSurface ? {hooks:__componentHooks} : __surfaceStates.get(id);
        if (state?.hooks) __nickelCleanupHooks(state.hooks);
        if (id === __activeSurface) {
            __componentHooks = new Map(); __componentRecords = new Map();
            __handlers = new __NickelHandlers(); __previousHandlers = new __NickelHandlers(); __handlerSlots = new Map();
            __effects = []; __dirtyComponents = new Set();
        } else if (state) {
            state.hooks = new Map(); state.records = new Map(); state.handlers = new __NickelHandlers();
            state.previousHandlers = new __NickelHandlers(); state.handlerSlots = new Map(); state.effects = []; state.dirty = new Set();
        }
    }
    __surfaceApps.set(id, component);
    __surfaceAppIdentities.set(id, {...identity,component});
    return compatible;
}

// Registrations retain executable values in this package's module graph. Only
// declarative metadata crosses the host boundary; package identity comes from Rust.
function __nickelResource(name, fallback) {
    __nickelMarkResourceRead(name);
    const value = __nickelProjectionField(__nickelData, name);
    return JSON.parse(JSON.stringify(value === undefined ? fallback : value));
}
function __nickelMarkResourceRead(name) {
    if (__currentComponent === null) return;
    let reads = __componentResourceReads.get(__currentComponent);
    if (!reads) __componentResourceReads.set(__currentComponent, reads = new Set());
    reads.add(name);
}
function __nickelDirtyResourceConsumers(name) {
    const dirty = (records, pending) => {
        for (const [path, record] of records)
            if (record.resources?.includes(name)) pending.add(path);
    };
    dirty(__componentRecords, __dirtyComponents);
    for (const state of __surfaceStates.values())
        if (state.records !== __componentRecords) dirty(state.records, state.dirty);
}
function __nickelIdentity(id) {
    if (typeof id !== 'string' || !id.length || id.length > 512) throw TypeError('invalid capability identity');
    return id;
}
// Ordinary application data/effects; an effect is a request for the trusted host.
const twinkle = Object.freeze({
    get data() { return __nickelData; },
    request(effect) { __effects.push(effect); },
    component(contract) {
        const component = __nickelPublicComponents.get(contract);
        if (!component) throw Error(`unknown public component ${contract}`);
        return component;
    }
});

// Immutable native mount inventories never escape directly. Compatibility
// fields are copied lazily on first public read; private resource reads can
// continue to clone the host value without exposing that retained inventory.
const __mountDataProjections = new WeakMap();
let __mountDataBase = null;
function __nickelCloneProjection(value) {
    if (value === null || typeof value !== 'object') return value;
    if (Array.isArray(value)) return value.map(__nickelCloneProjection);
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, __nickelCloneProjection(item)]));
}
function __nickelProjectionField(data, name) {
    const projection = __mountDataProjections.get(data);
    return projection ? (projection.exposed.has(name) ? projection.exposed.get(name) : projection.source[name]) : data[name];
}
function __nickelSetMountData({base, surface, hasSurface, props}) {
    if (base !== null) {
        __mountDataBase = base;
        __runtimeCounters.mountBaseUploads++;
    } else {
        __runtimeCounters.mountBaseReuses++;
    }
    if (__mountDataBase === null) throw Error('mount inventory is unavailable');
    const source = {...__mountDataBase, __componentProps:props};
    if (hasSurface) source.surface = surface;
    const exposed = new Map();
    const data = {};
    for (const key of Object.keys(source).sort()) {
        Object.defineProperty(data, key, {enumerable:true, get() {
            if (!exposed.has(key)) exposed.set(key, __nickelCloneProjection(source[key]));
            return exposed.get(key);
        }});
    }
    __mountDataProjections.set(data, {source, exposed});
    __nickelPublishData(data);
}
function __nickelSetData(data) {
    __mountDataBase = null;
    __nickelPublishData(data);
}
function __nickelPublishData(data) {
    const changed = new Map();
    const fieldChanged = name => {
        if (!changed.has(name)) {
            const previous = __nickelProjectionField(__nickelData, name);
            const next = __nickelProjectionField(data, name);
            changed.set(name, !Object.is(previous, next) && JSON.stringify(previous) !== JSON.stringify(next));
        }
        return changed.get(name);
    };
    const dirtyConsumers = (records, dirty) => {
        for (const [path, record] of records)
            if (record.resources?.some(fieldChanged)) dirty.add(path);
    };
    dirtyConsumers(__componentRecords, __dirtyComponents);
    for (const state of __surfaceStates.values())
        if (state.records !== __componentRecords) dirtyConsumers(state.records, state.dirty);
    __nickelData = Object.freeze(data);
}

function __nickelForEachSurfaceHooks(visit) {
    visit(__componentHooks, __dirtyComponents);
    for (const state of __surfaceStates.values())
        if (state.hooks !== __componentHooks) visit(state.hooks, state.dirty);
}

function __nickelSetLocaleStore(value){
    const before=__nickelDirtyComponentCount();
    if(!value||typeof value!=='object'||Array.isArray(value))throw Error('invalid locale snapshot');
    const known=value.known===true;
    let tag=known?value.tag:'und';
    if(typeof tag!=='string'||tag.length>64||!(/^[A-Za-z]{2,8}(?:-[A-Za-z0-9]{1,8})*$/.test(tag)))throw Error('invalid locale tag');
    const parts=tag.split('-');tag=parts.map((part,index)=>index===0?part.toLowerCase():part.length===2?part.toUpperCase():part.length===4?part[0].toUpperCase()+part.slice(1).toLowerCase():part.toLowerCase()).join('-');
    const direction=known?(value.direction??'ltr'):'ltr';if(direction!=='ltr'&&direction!=='rtl')throw Error('invalid locale direction');
    const previous=__localeStore.snapshot;if(previous.tag===tag&&previous.direction===direction&&previous.known===known)return false;
    if(__pendingRender!==null||__pendingEvent!==null)throw Error('cannot publish changed locale store during a render or event');
    const generation=__localeStore.generation+1,snapshot=Object.freeze({generation,tag,direction,known});__localeStore={generation,snapshot};__nickelNotifyExternalStore('locale');
    __nickelForEachSurfaceHooks((hooks,dirty)=>{for(const [owner,slots] of hooks)for(const entry of slots)if(entry?.kind==='locale-store'&&!Object.is(snapshot,entry.value))dirty.add(owner);});__nickelRecordStoreChange('locale',generation,before);return true;
}
function useLocale(){if(__currentComponent===null)throw Error('useLocale requires a component');const slot=__hookIndex++,hooks=__componentHooks.get(__currentComponent);let entry=hooks[slot];
    if(!entry)hooks[slot]=entry={kind:'locale-store',value:undefined,generation:0};if(entry.kind!=='locale-store')throw Error('hook order changed');entry.value=__localeStore.snapshot;entry.generation=__localeStore.generation;return entry.value;}
function useSyncExternalStore(subscribe,getSnapshot) {
    if(__currentComponent===null)throw Error('useSyncExternalStore requires a component');
    const store=__nickelExternalSubscriptions.get(subscribe),snapshotStore=__nickelExternalSnapshots.get(getSnapshot);
    if(!store||store!==snapshotStore)throw TypeError('useSyncExternalStore requires a matching branded host store contract');
    const slot=__hookIndex++,hooks=__componentHooks.get(__currentComponent);let entry=hooks[slot];
    if(!entry||(entry.kind==='sync-external-store'&&(entry.store!==store||entry.getSnapshot!==getSnapshot)))
        hooks[slot]=entry={kind:'sync-external-store',store,getSnapshot,value:undefined,generation:0};
    if(entry.kind!=='sync-external-store')throw Error('hook order changed');
    const before=store.read(),value=getSnapshot(),after=store.read();
    if(before.generation!==after.generation||!Object.is(value,after.snapshot))throw Error('external store changed during render');
    if(entry.generation===before.generation&&entry.value!==undefined&&!Object.is(value,entry.value))
        __nickelDeveloperDiagnostic('unstable-selector',`${store.name} external snapshot changed for an unchanged generation`,
            'Return the store snapshot directly; do not allocate a new snapshot from getSnapshot.');
    entry.storeError=undefined;entry.value=value;entry.generation=after.generation;return value;
}

function memo(component,compare) {
    if(typeof component!=='function')throw TypeError('memo component must be a function');
    if(compare!==undefined&&typeof compare!=='function')throw TypeError('memo comparison must be a function');
    const wrapped=props=>component(props);__memoComponents.set(wrapped,{component,compare:compare??null});return wrapped;
}

const __nickelThemePaletteFields = ['background','panel','surface','surfaceHover','text','muted',
    'accent','accentSoft','complement'];
function __nickelThemeColor(value, field) {
    if (value === undefined || value === null) return null;
    if (!Number.isSafeInteger(value) || value < 0 || value > 0xffffffff)
        throw Error(`invalid theme ${field}`);
    return value;
}
function __nickelThemeSnapshot(value, generation) {
    if (!value || typeof value !== 'object' || Array.isArray(value))
        throw Error('invalid effective theme snapshot');
    const mode = value.mode ?? 'unknown';
    if (mode !== 'light' && mode !== 'dark' && mode !== 'unknown')
        throw Error('invalid effective theme mode');
    const nullableBoolean = field => value[field] === undefined || value[field] === null ? null
        : typeof value[field] === 'boolean' ? value[field]
        : (() => { throw Error(`invalid theme ${field}`); })();
    const boundedInteger = (field, maximum) => value[field] === undefined || value[field] === null ? null
        : Number.isInteger(value[field]) && value[field] >= 0 && value[field] <= maximum ? value[field]
        : (() => { throw Error(`invalid theme ${field}`); })();
    let palette = null;
    if (value.palette !== undefined && value.palette !== null) {
        if (typeof value.palette !== 'object' || Array.isArray(value.palette))
            throw Error('invalid theme palette');
        const colors = {};
        for (const field of __nickelThemePaletteFields) {
            const color = __nickelThemeColor(value.palette[field], `palette ${field}`);
            if (color === null) throw Error(`missing theme palette ${field}`);
            colors[field] = color;
        }
        palette = Object.freeze(colors);
    }
    return Object.freeze({generation, mode, accent:__nickelThemeColor(value.accent, 'accent'),
        accentHue:boundedInteger('accentHue', 359), accentIntensity:boundedInteger('accentIntensity', 100),
        reducedMotion:nullableBoolean('reducedMotion'),
        reducedTransparency:nullableBoolean('reducedTransparency'), palette});
}
function __nickelThemeEqual(left, right) {
    const fields = ['mode','accent','accentHue','accentIntensity','reducedMotion','reducedTransparency'];
    if (!fields.every(field => Object.is(left[field], right[field]))) return false;
    if (left.palette === null || right.palette === null) return left.palette === right.palette;
    return __nickelThemePaletteFields.every(field => left.palette[field] === right.palette[field]);
}
function __nickelSetThemeStore(value) {
    const before=__nickelDirtyComponentCount();
    const generation = __themeStore.generation + 1;
    let snapshot = __nickelThemeSnapshot(value, generation);
    if (__nickelThemeEqual(__themeStore.snapshot, snapshot)) return false;
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish changed theme store during a render or event');
    if (__themeStore.snapshot.palette !== null && snapshot.palette !== null
        && __nickelThemePaletteFields.every(field => __themeStore.snapshot.palette[field] === snapshot.palette[field]))
        snapshot = Object.freeze({...snapshot, palette:__themeStore.snapshot.palette});
    __themeStore = {generation, snapshot};
    __nickelNotifyExternalStore('theme');
    __nickelForEachSurfaceHooks((hooks, dirty) => {
        for (const [owner, slots] of hooks) for (const entry of slots) {
            if (entry?.kind !== 'theme-store') continue;
            try {
                const selected = entry.selector ? entry.selector(snapshot) : snapshot;
                entry.storeError = undefined;
                if (!Object.is(selected, entry.value)) dirty.add(owner);
            } catch (error) {
                entry.storeError = error;
                dirty.add(owner);
            }
        }
    });
    __nickelRecordStoreChange('theme',generation,before); return true;
}
function useTheme(selector) {
    if (__currentComponent === null) throw Error('useTheme requires a component');
    if (selector !== undefined && typeof selector !== 'function') throw TypeError('useTheme selector must be a function');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    const normalized = selector ?? null;
    const entry=__nickelSelectedStoreEntry(hooks,slot,'theme-store',normalized,'theme');
    entry.storeError = undefined;
    entry.value = __nickelSelectedValue(entry,normalized,__themeStore.snapshot,__themeStore.generation,'theme');
    entry.generation = __themeStore.generation;
    return entry.value;
}
const __nickelSelectReducedMotion = theme => theme.reducedMotion;
function useReducedMotion() { return useTheme(__nickelSelectReducedMotion); }

function __nickelSetCapabilityStore(value) {
    const before=__nickelDirtyComponentCount();
    if (!value || typeof value !== 'object' || Array.isArray(value)
        || !Array.isArray(value.known) || !value.entries || typeof value.entries !== 'object'
        || Array.isArray(value.entries) || value.known.length > 128)
        throw Error('invalid capability store snapshot');
    const known = Object.freeze(value.known.map(name => {
        if (typeof name !== 'string' || !name.length || name.length > 64) throw Error('invalid capability name');
        return name;
    }));
    if (new Set(known).size !== known.length) throw Error('duplicate capability name');
    const entries = {};
    for (const name of known) {
        const source = value.entries[name] ?? {};
        const available = source.available === undefined || source.available === null ? null
            : typeof source.available === 'boolean' ? source.available
            : (() => { throw Error('invalid capability availability'); })();
        const reason = source.reason === undefined || source.reason === null ? null
            : typeof source.reason === 'string' && source.reason.length <= 480 ? source.reason
            : (() => { throw Error('invalid capability reason'); })();
        entries[name] = Object.freeze({declared:source.declared === true, available, reason});
    }
    const previous = __capabilityStore;
    const unchanged = previous.known.length === known.length
        && previous.known.every((name, index) => name === known[index])
        && known.every(name => {
            const left = previous.snapshot[name], right = entries[name];
            return left && left.declared === right.declared && left.available === right.available && left.reason === right.reason;
        });
    if (unchanged) return false;
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish changed capability store during a render or event');
    for (const name of known) {
        const prior = previous.snapshot[name], next = entries[name];
        if (prior && prior.declared === next.declared && prior.available === next.available && prior.reason === next.reason)
            entries[name] = prior;
    }
    const generation = previous.generation + 1;
    __capabilityStore = {generation, known, snapshot:Object.freeze(entries)};
    __nickelNotifyExternalStore('capabilities');
    __nickelForEachSurfaceHooks((hooks, dirty) => {
        for (const [owner, slots] of hooks) for (const entry of slots)
            if (entry?.kind === 'capability-store' && !Object.is(entries[entry.capability], entry.value)) dirty.add(owner);
    });
    __nickelRecordStoreChange('capabilities',generation,before); return true;
}
function useHostCapability(capability) {
    if (__currentComponent === null) throw Error('useHostCapability requires a component');
    if (typeof capability !== 'string' || !__capabilityStore.known.includes(capability))
        throw TypeError('unknown host capability');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    let entry = hooks[slot];
    if (!entry || (entry.kind === 'capability-store' && entry.capability !== capability))
        hooks[slot] = entry = {kind:'capability-store', capability, value:undefined, generation:0};
    if (entry.kind !== 'capability-store') throw Error('hook order changed');
    entry.value = __capabilityStore.snapshot[capability];
    entry.generation = __capabilityStore.generation;
    return entry.value;
}

function __nickelSurfaceSelection(name, snapshot) {
    if (name === 'surface') return snapshot;
    if (name === 'output') return snapshot.output;
    if (name === 'scale') return snapshot.scaleFactor;
    if (name === 'focus') return snapshot.focused;
    throw Error('unknown surface store selection');
}

function __nickelSetSurfaceStore(mountId, value) {
    const before=__nickelDirtyComponentCount();
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot publish surface store during a render or event');
    if (typeof mountId !== 'string' || !mountId.length || mountId.length > 128)
        throw Error('invalid native surface mount identity');
    if (!value || typeof value !== 'object' || Array.isArray(value))
        throw Error('invalid native surface snapshot');
    const nullableString = field => value[field] === undefined || value[field] === null
        ? null : typeof value[field] === 'string' ? value[field] : (() => { throw Error(`invalid surface ${field}`); })();
    const nullableBoolean = field => value[field] === undefined || value[field] === null
        ? null : typeof value[field] === 'boolean' ? value[field] : (() => { throw Error(`invalid surface ${field}`); })();
    const size = (width, height) => Number.isFinite(width) && width >= 0 && Number.isFinite(height) && height >= 0
        ? Object.freeze({width, height}) : null;
    const next = {
        mountId,
        id:nullableString('id'),
        kind:nullableString('kind'),
        logicalSize:size(value.width, value.height),
        output:nullableString('output'),
        availableSize:size(value.availableWidth, value.availableHeight),
        scaleFactor:value.scaleFactor === undefined || value.scaleFactor === null ? null
            : Number.isFinite(value.scaleFactor) && value.scaleFactor > 0 ? value.scaleFactor
            : (() => { throw Error('invalid surface scaleFactor'); })(),
        focused:nullableBoolean('focused'),
        visible:nullableBoolean('visible')
    };
    const previous = __surfaceStore.snapshot;
    const sameSize = (left, right) => left === right || (left !== null && right !== null
        && left.width === right.width && left.height === right.height);
    const unchanged = previous.mountId === next.mountId && previous.id === next.id
        && previous.kind === next.kind && sameSize(previous.logicalSize, next.logicalSize)
        && previous.output === next.output && sameSize(previous.availableSize, next.availableSize)
        && Object.is(previous.scaleFactor, next.scaleFactor) && previous.focused === next.focused
        && previous.visible === next.visible;
    if (unchanged) return false;
    if (sameSize(previous.logicalSize,next.logicalSize)) next.logicalSize=previous.logicalSize;
    if (sameSize(previous.availableSize,next.availableSize)) next.availableSize=previous.availableSize;
    const generation = __surfaceStore.generation + 1;
    const snapshot = Object.freeze({...next, generation});
    __surfaceStore = {generation, snapshot};
    for (const [owner, hooks] of __componentHooks) {
        for (const entry of hooks) {
            if (entry?.kind !== 'surface-store') continue;
            const selected = entry.selector ? entry.selector(snapshot) : __nickelSurfaceSelection(entry.selection, snapshot);
            if (!Object.is(selected, entry.value)) __dirtyComponents.add(owner);
        }
    }
    __nickelRecordStoreChange('surface',generation,before); return true;
}

function __nickelUseSurfaceSelection(selection, selector) {
    if (__currentComponent === null) throw Error('surface hooks require a component');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    let entry = hooks[slot];
    if (!entry) hooks[slot] = entry = {kind:'surface-store', selection, selector:null, value:undefined, generation:0};
    if (entry.kind !== 'surface-store' || entry.selection !== selection) throw Error('hook order changed');
    entry.selector=selector??null;
    entry.value = entry.selector ? entry.selector(__surfaceStore.snapshot) : __nickelSurfaceSelection(selection, __surfaceStore.snapshot);
    entry.generation = __surfaceStore.generation;
    return entry.value;
}

function useSurface(selector) { if(selector!==undefined&&typeof selector!=='function')throw TypeError('useSurface selector must be a function'); return __nickelUseSurfaceSelection('surface',selector); }
let __twinkleOutputResolver = () => null;
function useOutput() { return __twinkleOutputResolver(__nickelUseSurfaceSelection('output')); }
function useScaleFactor() { return __nickelUseSurfaceSelection('scale'); }
function useSurfaceFocus() { return __nickelUseSurfaceSelection('focus'); }

function __nickelSelectSurface(id) {
    if (typeof id !== 'string' || !id.length) throw Error('invalid surface identity');
    if (__pendingRender !== null || __pendingEvent !== null)
        throw Error('cannot switch surfaces during a render or event');
    if (id === __activeSurface) return;
    if (__activeSurface) __surfaceStates.set(__activeSurface, {
        hooks: __componentHooks,
        records: __componentRecords,
        handlers: __handlers,
        previousHandlers: __previousHandlers,
        handlerSlots: __handlerSlots,
        effects: __effects,
        data: __nickelData,
        dirty: __dirtyComponents,
        surfaceStore: __surfaceStore,
        acceptedNativeTree: __acceptedNativeTree
    });
    const state = __surfaceStates.get(id);
    __componentHooks = state?.hooks ?? new Map();
    __componentRecords = state?.records ?? new Map();
    __handlers = state?.handlers ?? new __NickelHandlers();
    __previousHandlers = state?.previousHandlers ?? new __NickelHandlers();
    __handlerSlots = state?.handlerSlots ?? new Map();
    __effects = state?.effects ?? [];
    __dirtyComponents = state?.dirty ?? new Set();
    __surfaceStore = state?.surfaceStore ?? {generation:0, snapshot:Object.freeze({
        generation:0, mountId:null, id:null, kind:null, logicalSize:null,
        output:null, availableSize:null, scaleFactor:null, focused:null, visible:null
    })};
    __acceptedNativeTree = state?.acceptedNativeTree ?? null;
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
    const retired = id === __activeSurface ? __componentHooks : __surfaceStates.get(id)?.hooks;
    if (retired) __nickelCleanupHooks(retired);
    __surfaceStates.delete(id);
    __mountProfiles.delete(id);
    __surfaceApps.delete(id);
    __surfaceAppIdentities.delete(id);
    if (id !== __activeSurface) return;
    __componentHooks = new Map();
    __componentRecords = new Map();
    __handlers = new __NickelHandlers();
    __previousHandlers = new __NickelHandlers();
    __handlerSlots = new Map();
    __effects = [];
    __dirtyComponents = new Set();
    __acceptedNativeTree = null;
    __surfaceStore = {generation:0, snapshot:Object.freeze({
        generation:0, mountId:null, id:null, kind:null, logicalSize:null,
        output:null, availableSize:null, scaleFactor:null, focused:null, visible:null
    })};
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
    const owner = __currentComponent;
    const surface = __activeSurface;
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(owner);
    if (!hooks[slot]) {
        const entry = {kind: 'state', value: typeof initial === 'function' ? initial() : initial, set: null};
        entry.set = next => {
            const state = surface === __activeSurface
                ? {hooks:__componentHooks, dirty:__dirtyComponents}
                : __surfaceStates.get(surface);
            const current = state?.hooks.get(owner)?.[slot];
            // Surface IDs and component paths can be reused after retirement.
            // Only this exact accepted hook lifetime may receive the update.
            if (!current || current.kind !== 'state' || current.set !== entry.set) return;
            if (__currentComponent !== null)
                throw Error('state updates are not allowed during component render');
            const value = typeof next === 'function' ? next(current.value) : next;
            if (!Object.is(value, current.value)) {
                current.value = value;
                state.dirty.add(owner);
            }
        };
        hooks[slot] = entry;
    }
    if (hooks[slot].kind !== 'state') throw Error('hook order changed');
    const entry = hooks[slot];
    return [entry.value, entry.set];
}

function useReducer(reducer, initialArg, init) {
    if (__currentComponent === null) throw Error('useReducer requires a component');
    if (typeof reducer !== 'function') throw TypeError('useReducer requires a reducer');
    if (init !== undefined && typeof init !== 'function') throw TypeError('useReducer initializer must be a function');
    const owner = __currentComponent;
    const surface = __activeSurface;
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(owner);
    if (!hooks[slot]) {
        const entry = {kind: 'reducer', value: init === undefined ? initialArg : init(initialArg), reducer, dispatch: null};
        entry.dispatch = action => {
            const state = surface === __activeSurface
                ? {hooks:__componentHooks, dirty:__dirtyComponents}
                : __surfaceStates.get(surface);
            const current = state?.hooks.get(owner)?.[slot];
            if (!current || current.kind !== 'reducer' || current.dispatch !== entry.dispatch) return;
            if (__currentComponent !== null)
                throw Error('reducer updates are not allowed during component render');
            let value;
            try { value = current.reducer(current.value, action); }
            catch (error) {
                if (__nickelCaptureFailure(owner, error, 'reducer', surface)) return;
                throw error;
            }
            if (!Object.is(value, current.value)) {
                current.value = value;
                state.dirty.add(owner);
            }
        };
        hooks[slot] = entry;
    }
    if (hooks[slot].kind !== 'reducer') throw Error('hook order changed');
    const entry = hooks[slot];
    entry.nextReducer = reducer;
    __pendingRender.reducerEntries.push(entry);
    return [entry.value, entry.dispatch];
}

function useRef(initial) {
    if (__currentComponent === null) throw Error('useRef requires a component');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    if (!hooks[slot]) hooks[slot] = {kind: 'ref', value: {current: initial}};
    if (hooks[slot].kind !== 'ref') throw Error('hook order changed');
    return hooks[slot].value;
}

function useId() {
    if (__currentComponent === null) throw Error('useId requires a component');
    const owner = __currentComponent;
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(owner);
    if (!hooks[slot]) {
        const surface = encodeURIComponent(__activeSurface);
        const component = encodeURIComponent(owner);
        hooks[slot] = {kind: 'id', value: `:nickel:${surface}:${component}:${slot}:`};
    }
    if (hooks[slot].kind !== 'id') throw Error('hook order changed');
    return hooks[slot].value;
}

function createContext(defaultValue) {
    const context = {};
    function Provider() {
        throw Error('Context.Provider can only be rendered as a component');
    }
    Object.defineProperties(context, {
        Provider:{value:Provider, enumerable:true},
        defaultValue:{value:defaultValue}
    });
    Object.freeze(context);
    __contexts.add(context);
    __contextProviders.set(Provider, context);
    return context;
}

function __nickelIsContext(value) {
    return value !== null && typeof value === 'object' && __contexts.has(value);
}

function useContext(context) {
    if (__currentComponent === null) throw Error('useContext requires a component');
    if (!__nickelIsContext(context)) throw TypeError('useContext requires a Nickel context');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    let entry = hooks[slot];
    const values = __contextValues.get(context);
    const current = values?.length ? values[values.length - 1] : null;
    if (!entry || (entry.kind === 'context' && entry.provider !== current?.provider))
        hooks[slot] = entry = {kind:'context', context, provider:current?.provider ?? null, value:context.defaultValue};
    if (entry.kind !== 'context' || entry.context !== context) throw Error('hook order changed');
    entry.value = current ? current.value : context.defaultValue;
    return entry.value;
}

function __nickelDepsEqual(left, right) {
    return left !== undefined && right !== undefined && left.length === right.length
        && left.every((value, index) => Object.is(value, right[index]));
}

function __nickelDeps(deps, hook) {
    if (deps !== undefined && !Array.isArray(deps)) throw TypeError(`${hook} dependencies must be an array`);
    return deps === undefined ? undefined : deps.slice();
}

function useMemo(factory, deps) {
    if (__currentComponent === null) throw Error('useMemo requires a component');
    if (typeof factory !== 'function') throw TypeError('useMemo requires a factory');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    const nextDeps = __nickelDeps(deps, 'useMemo');
    const previous = hooks[slot];
    if (previous && previous.kind !== 'memo') throw Error('hook order changed');
    if (previous && __nickelDepsEqual(previous.deps, nextDeps)) return previous.value;
    const value = factory();
    hooks[slot] = {kind: 'memo', value, deps: nextDeps};
    return value;
}

function useCallback(callback, deps) {
    if (typeof callback !== 'function') throw TypeError('useCallback requires a function');
    return useMemo(() => callback, deps);
}

function useEffect(setup, deps) {
    if (__currentComponent === null) throw Error('useEffect requires a component');
    if (typeof setup !== 'function') throw TypeError('useEffect requires a setup function');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    const nextDeps = __nickelDeps(deps, 'useEffect');
    let entry = hooks[slot];
    if (!entry) hooks[slot] = entry = {kind: 'effect', owner:__currentComponent, deps: undefined, cleanup: undefined};
    if (entry.kind !== 'effect') throw Error('hook order changed');
    if (!__nickelDepsEqual(entry.deps, nextDeps)) {
        __runtimeCounters.effectsScheduled++;
        __pendingRender.passiveEffects.push({entry, setup, deps: nextDeps});
    }
}

function __nickelRunCleanup(entry) {
    if (typeof entry.cleanup !== 'function') return;
    const cleanup = entry.cleanup;
    entry.cleanup = undefined;
    __runtimeCounters.cleanups++;
    try { cleanup(); }
    catch (error) { __nickelCaptureFailure(entry.owner, error, 'cleanup'); }
}

function __nickelCleanupHooks(hooks) {
    for (const slots of hooks.values())
        for (const entry of slots)
            if (entry?.kind === 'effect') __nickelRunCleanup(entry);
}

// Function components are declarations until reconciliation. The WeakSet brand
// is intentionally not serializable and never crosses the native boundary.
const __componentDeclarations = new WeakSet();

function __nickelComponentDeclaration(component, props, children) {
    const declaration = {};
    Object.defineProperties(declaration, {
        component:{value:component},
        props:{value:props ?? null},
        children:{value:children},
        key:{value:props?.key},
        toJSON:{value:()=>{ throw Error('unresolved component declaration cannot be serialized'); }}
    });
    Object.freeze(declaration);
    __componentDeclarations.add(declaration);
    return declaration;
}

function __nickelIsComponentDeclaration(value) {
    return value !== null && typeof value === 'object' && __componentDeclarations.has(value);
}

function __nickelMarkNonRetainedComponent(component) {
    __nonRetainedComponents.add(component);
    return component;
}

function __nickelSameDeclaration(previous, next) {
    if (!previous || previous.component !== next.component || previous.key !== next.key) return false;
    const left = previous.props ?? {};
    const right = next.props ?? {};
    const leftKeys = Object.keys(left);
    const rightKeys = Object.keys(right);
    if (leftKeys.length !== rightKeys.length) return false;
    for (const key of leftKeys)
        if (!Object.prototype.hasOwnProperty.call(right, key) || !Object.is(left[key], right[key]))
            return false;
    if (previous.children.length !== next.children.length) return false;
    return previous.children.every((child, index) => Object.is(child, next.children[index]));
}

function __nickelMemoProps(declaration) {
    const props={...declaration.props};
    if(declaration.children.length===1)props.children=declaration.children[0];
    else if(declaration.children.length>1)props.children=declaration.children;
    return props;
}
function __nickelMemoSame(previous,next,configuration) {
    if(!previous||previous.component!==next.component||previous.key!==next.key)return false;
    const left=__nickelMemoProps(previous),right=__nickelMemoProps(next);
    if(configuration.compare)return configuration.compare(left,right)===true;
    const leftKeys=Object.keys(left),rightKeys=Object.keys(right);return leftKeys.length===rightKeys.length
        &&leftKeys.every(key=>Object.prototype.hasOwnProperty.call(right,key)&&Object.is(left[key],right[key]));
}

function __nickelDirtyAtOrBelow(path) {
    if (!__incrementalRender) return true;
    for (const dirty of __dirtyComponents)
        if (dirty === path || dirty.startsWith(`${path}/`)) return true;
    return false;
}

function __nickelMarkRetainedVisited(path) {
    const pending = [path];
    while (pending.length) {
        const retained = pending.pop();
        __visitedComponents.add(retained);
        const children = __retainedComponentChildren.get(retained);
        if (children) pending.push(...children);
    }
}

function __nickelIndexRetainedComponentChildren(records) {
    const children = new Map();
    for (const path of records.keys()) {
        const identity = path.lastIndexOf('/');
        const component = identity < 0 ? -1 : path.lastIndexOf('/', identity - 1);
        if (component < 0) continue;
        const parent = path.slice(0, component);
        if (parent === 'root') continue;
        const siblings = children.get(parent);
        if (siblings) siblings.push(path);
        else children.set(parent, [path]);
    }
    return children;
}

function __nickelResolveDeclaration(declaration) {
    const kind = declaration.component;
    let type = __componentIds.get(kind);
    if (type === undefined) {
        type = ++__nextComponentId;
        __componentIds.set(kind, type);
    }
    const parent = __currentComponent ?? 'root';
    const ordinalKey = `${parent}/${type}`;
    const ordinal = __componentChildren.get(ordinalKey) ?? 0;
    __componentChildren.set(ordinalKey, ordinal + 1);
    const identity = declaration.key === undefined ? `#${ordinal}`
        : `@${encodeURIComponent(String(declaration.key))}`;
    const path = `${ordinalKey}/${identity}`;
    if (__visitedComponents.has(path)) throw Error(`duplicate component key ${identity}`);
    __visitedComponents.add(path);
    if (!__componentHooks.has(path)) __componentHooks.set(path, []);
    const retained = __componentRecords.get(path);
    const memoConfiguration=__memoComponents.get(kind);
    const declarationSame=memoConfiguration?__nickelMemoSame(retained?.declaration,declaration,memoConfiguration)
        :__nickelSameDeclaration(retained?.declaration,declaration);
    const execute = !__incrementalRender || !retained || __nonRetainedComponents.has(kind)
        || __dirtyComponents.has(path) || !declarationSame;
    if (!execute && !__nickelDirtyAtOrBelow(path)) {
        __runtimeCounters.reused++;
        __nickelMarkRetainedVisited(path);
        return retained.output;
    }
    const previous = __currentComponent;
    const previousIndex = __hookIndex;
    __currentComponent = path;
    const componentMetadata=__nickelComponentMetadata.get(kind);
    __componentExecutionStack.push({path,module:componentMetadata?.module??null,
        export:componentMetadata?.export??kind.name??null,kind});
    __runtimeCounters.executed++;
    const profile=__nickelProfile();
    const recursiveExecutions=__componentExecutionStack.filter(entry=>entry.kind===kind).length;
    if(recursiveExecutions===33)
        __nickelDeveloperDiagnostic('render-loop','the same component recursively rendered 33 times in one reconciliation',
            'Add a bounded termination condition or replace recursive composition with an iterative child list.',path,
            {recursiveExecutions});
    const reason = !retained ? 'mount' : __dirtyComponents.has(path) ? 'hook-or-store'
        : !declarationSame ? 'props' : 'descendant';
    if (__runtimeReasons.length === __MAX_RUNTIME_REASONS) __runtimeReasons.shift();
    __runtimeReasons.push({surface:__activeSurface,path,reason});
    __hookIndex = 0;
    if (execute) __componentResourceReads.set(path, new Set());
    const componentDepth=__componentExecutionStack.length;
    if(componentDepth>48)
        __nickelDeveloperDiagnostic('excessive-component-depth',`component depth ${componentDepth} exceeds the development threshold`,
            'Flatten wrapper components or split the surface into smaller retained boundaries.',path,{depth:componentDepth});
    try {
        if (__errorBoundaryComponents.has(kind)) {
            const resetKeys = declaration.props?.resetKeys;
            if (resetKeys !== undefined && !Array.isArray(resetKeys))
                throw TypeError('ErrorBoundary resetKeys must be an array');
            let boundaryError = retained?.boundaryError;
            if (boundaryError !== undefined && resetKeys !== undefined
                && !__nickelDepsEqual(retained?.resetKeys, resetKeys))
                boundaryError = undefined;
            const reset = () => {
                const current = __componentRecords.get(path);
                if (!current?.boundary) return;
                __componentRecords.set(path, {...current, boundaryError:undefined});
                __dirtyComponents.add(path);
            };
            const resolveFallback = error => {
                const fallback = declaration.props?.fallback ?? null;
                const raw = typeof fallback === 'function' ? fallback(error, reset) : fallback;
                return {raw, output:__nickelApplyDeclarationKey(declaration, __nickelResolveVirtual(raw))};
            };
            if (boundaryError !== undefined) {
                const failed = resolveFallback(boundaryError);
                __componentRecords.set(path, {kind,declaration,...failed,boundary:true,boundaryError,resetKeys:resetKeys?.slice()});
                return failed.output;
            }
            __componentRecords.set(path, {kind,declaration,raw:null,output:retained?.output ?? null,boundary:true,boundaryError:undefined,resetKeys:resetKeys?.slice()});
            try {
                const raw = declaration.children.length === 1 ? declaration.children[0] : declaration.children;
                const output = __nickelApplyDeclarationKey(declaration, __nickelResolveVirtual(raw));
                __componentRecords.set(path, {kind,declaration,raw,output,boundary:true,boundaryError:undefined,resetKeys:resetKeys?.slice()});
                return output;
            } catch (error) {
                __nickelRecordBoundaryFailure(path, 'render', error);
                const failed = resolveFallback(error);
                __componentRecords.set(path, {kind,declaration,...failed,boundary:true,boundaryError:error,resetKeys:resetKeys?.slice()});
                return failed.output;
            }
        }
        const context = __contextProviders.get(kind);
        let raw;
        if (context) {
            const values = __contextValues.get(context) ?? [];
            if (!__contextValues.has(context)) __contextValues.set(context, values);
            const value = declaration.props?.value;
            for (const [owner, hooks] of __componentHooks)
                for (const entry of hooks)
                    if (entry?.kind === 'context' && entry.context === context
                        && entry.provider === path && !Object.is(entry.value, value))
                        __dirtyComponents.add(owner);
            values.push({provider:path, value});
            try {
                raw = declaration.children.length === 1 ? declaration.children[0] : declaration.children;
                const result = __nickelResolveVirtual(raw);
                const output = __nickelApplyDeclarationKey(declaration, result);
                const resources = Array.from(__componentResourceReads.get(path) ?? retained?.resources ?? []);
                __componentResourceReads.delete(path);
                __componentRecords.set(path, {kind, declaration, raw, output,
                    resources});
                return output;
            } finally {
                values.pop();
                if (!values.length) __contextValues.delete(context);
            }
        } else {
            let resources = retained?.resources ?? [];
            if (execute) {
                const componentStarted=Date.now();profile.componentExecutions++;
                try { raw = kind({...declaration.props, children:declaration.children}); }
                catch (error) {
                    const metadata = __nickelComponentMetadata.get(kind);
                    try { Object.defineProperty(error, '__nickelComponent', {value:{path,module:metadata?.module ?? null,export:metadata?.export ?? kind.name ?? null}}); }
                    catch (_) { /* diagnostics must not replace the component failure */ }
                    throw error;
                } finally {
                    resources = Array.from(__componentResourceReads.get(path) ?? []);
                    __componentResourceReads.delete(path);
                    profile.componentExecutionMillis+=Math.max(0,Date.now()-componentStarted);
                }
            } else raw = retained.raw;
            const result = __nickelResolveVirtual(raw);
            if (execute && __hookIndex !== __componentHooks.get(path).length) throw Error('hook order changed');
            const output = __nickelApplyDeclarationKey(declaration, result);
            __componentRecords.set(path, {kind, declaration, raw, output, resources});
            return output;
        }
    } catch(error) {
        if(error instanceof TypeError||error instanceof RangeError)
            __nickelDeveloperDiagnostic('invalid-props',__nickelErrorMessage(error),
                'Check this component’s declared prop types and bounded native values.',path);
        throw error;
    } finally {
        __componentExecutionStack.pop();
        __currentComponent = previous;
        __hookIndex = previousIndex;
    }
}

function __nickelApplyDeclarationKey(declaration, node) {
    if (declaration.key === undefined || node === null || typeof node !== 'object' || Array.isArray(node))
        return node;
    const keyed = {...node, key:declaration.key};
    if (!__virtualNativeNodes.has(node)) return keyed;
    const result=__nickelVirtualNativeNode(keyed);
    if (__resolvedVirtualValues.has(node)) __resolvedVirtualValues.add(result);
    return result;
}

function __nickelResolveVirtual(value) {
    if (__nickelIsComponentDeclaration(value)) return __nickelResolveDeclaration(value);
    if (__nickelIsHandlerBinding(value)) return value;
    if (value && typeof value === 'object' && __resolvedVirtualValues.has(value)) return value;
    if (Array.isArray(value)) {
        const resolved=value.map(__nickelResolveVirtual);__resolvedVirtualValues.add(resolved);return resolved;
    }
    if (value && typeof value === 'object') {
        const resolved = {};
        for (const [key, item] of Object.entries(value)) resolved[key] = __nickelResolveVirtual(item);
        const result=__virtualNativeNodes.has(value)?__nickelVirtualNativeNode(resolved):resolved;
        __resolvedVirtualValues.add(result);return result;
    }
    return value;
}

function __nickelVirtualNativeNode(node) {
    Object.defineProperty(node, 'toJSON', {
        value:()=>{ throw Error('unmaterialized native node cannot be serialized'); }
    });
    __virtualNativeNodes.add(node);
    return node;
}

function __nickelHandlerBinding(handler) {
    const binding = Object.create(null);
    Object.defineProperties(binding, {
        handler:{value:handler},
        owner:{value:__currentComponent},
        toJSON:{value:()=>{ throw Error('unmaterialized event binding cannot be serialized'); }}
    });
    Object.freeze(binding);
    __handlerBindings.add(binding);
    return binding;
}

function __nickelIsHandlerBinding(value) {
    return value !== null && typeof value === 'object' && __handlerBindings.has(value);
}

function __nickelMaterializeVirtual(value, path = 'root') {
    if (__nickelIsHandlerBinding(value)) {
        const handler = value.handler, owner = value.owner;
        let action = __handlerSlots.get(path);
        if (action === undefined) {
            action = __handlers.allocate();
            __handlerSlots.set(path, action);
        }
        __handlers.set(action, input => {
            try { return handler(input); }
            catch (error) {
                if (!__nickelCaptureFailure(owner, error, 'event')) throw error;
            }
        });
        return action;
    }
    if (__nickelIsComponentDeclaration(value))
        throw Error('unresolved component declaration reached native materialization');
    if (Array.isArray(value)) {
        const node = value.map((item, index) =>
            __nickelMaterializeVirtual(item, `${path}/#${index}`));
        __nativeMaterializations.set(value, {node,path});
        return node;
    }
    if (value && typeof value === 'object') {
        // A retained virtual native node already has an admitted immutable
        // native representation. Key-derived paths stay stable across list
        // insertion, removal, and reorder, so reuse it without walking or
        // allocating its native subtree again.
        const admitted = __patchOnlyRender && __virtualNativeNodes.has(value)
            ? __nativeMaterializations.get(value) : undefined;
        if (admitted?.path === path) return admitted.node;
        const materialized = {};
        const native = __virtualNativeNodes.has(value);
        const slots = {};
        for (const [key, item] of Object.entries(value)) {
            if (native && key === 'children' && Array.isArray(item)) {
                materialized[key] = item.map((child, index) => {
                    const identity = child?.key === undefined ? `#${index}`
                        : `@${encodeURIComponent(String(child.key))}`;
                    return __nickelMaterializeVirtual(child, `${path}/${identity}`);
                });
            } else {
                const itemPath = __nickelIsHandlerBinding(item)
                    ? `${path}:${key}` : `${path}/${key}`;
                materialized[key] = __nickelMaterializeVirtual(item, itemPath);
            }
            if (__nickelIsHandlerBinding(item)) slots[key] = `${path}:${key}`;
        }
        if (native) {
            __runtimeCounters.nativeNodesMaterialized++;
            materialized.__nativeId = path;
            __nativeMaterializations.set(value, {node:materialized,path});
        }
        if (Object.keys(slots).length) materialized.__handlerSlots = slots;
        return materialized;
    }
    return value;
}

// Virtual output stays private to the JavaScript runtime. This weak association
// records the native subtree admitted for a component without retaining a
// second complete native tree or making virtual nodes serializable.
const __nativeMaterializations = new WeakMap();

function __nickelAttachNativeRecords(boundaries = null) {
    for (const [path, record] of __componentRecords) {
        if (boundaries !== null && !boundaries.some(boundary =>
            path === boundary || path.startsWith(`${boundary}/`))) continue;
        const admitted = record.output && typeof record.output === 'object'
            ? __nativeMaterializations.get(record.output) : undefined;
        if (admitted) __componentRecords.set(path, {...record,native:admitted.node,nativePath:admitted.path});
    }
}

const __nickelNativeProps = new Set([
    'id','title','className','open','anchor','placement','output','edge','reserveWorkArea','bottomOffset',
    'x','y','width','height','grow','background','padding','radius','color','label','disabledReason',
    'shortcut','separatorBefore','selected','hovered','dragging','outline','hoverBackground',
    'selectedBackground','accent','complement','item','count','hue','custom','asset','fit',
    'accessibilityLabel','role','aria-label','aria-checked','aria-selected','state','disabled','icon',
    'description','showLabel','iconSize','iconPlacement','value','placeholder','secure','autoFocus',
    'wrap','maxLines','percent','collection'
]);

function h(kind, props, ...children) {
    if (typeof kind === 'function') return __nickelComponentDeclaration(kind, props, children);
    for (const child of children) {
        if (!Array.isArray(child) || __fragmentChildren.has(child)) continue;
        const seen = new Set();
        for (const item of child.flat(Infinity).filter(item => item !== null && item !== false)) {
            if (typeof item !== 'object') continue;
            if (item?.key === undefined) {
                __listKeyErrors.push('items rendered from an array need a stable key');
                __nickelDeveloperDiagnostic('positional-identity-churn','dynamic children are using positional identity',
                    'Add a stable key to every component or native node produced by an array.');
                continue;
            }
            const key = String(item.key);
            if (seen.has(key)) __listKeyErrors.push(`duplicate list key ${key}`);
            seen.add(key);
        }
    }
    const handler = typeof props?.onClick === 'function' ? props.onClick : props?.onChange;
    const action = typeof handler === 'function' ? __nickelHandlerBinding(handler) : null;
    const contextAction = typeof props?.onContextMenu === 'function'
        ? __nickelHandlerBinding(props.onContextMenu) : null;
    const dragAction = typeof props?.onDrag === 'function'
        ? __nickelHandlerBinding(props.onDrag) : null;
    const dropAction = typeof props?.onDrop === 'function'
        ? __nickelHandlerBinding(props.onDrop) : null;
    const focusAction = typeof props?.onFocus === 'function'
        ? __nickelHandlerBinding(props.onFocus) : null;
    const blurAction = typeof props?.onBlur === 'function'
        ? __nickelHandlerBinding(props.onBlur) : null;
    const selectAction = typeof props?.onSelect === 'function'
        ? __nickelHandlerBinding(props.onSelect) : null;
    const moveAction = typeof props?.onMove === 'function'
        ? __nickelHandlerBinding(props.onMove) : null;
    const fileAction = typeof props?.onFileAction === 'function'
        ? __nickelHandlerBinding(props.onFileAction) : null;
    const closeAction = typeof props?.onClose === 'function'
        ? __nickelHandlerBinding(props.onClose) : null;
    const escapeAction = typeof props?.onEscape === 'function'
        ? __nickelHandlerBinding(props.onEscape) : null;
    const submitAction = typeof props?.onSubmit === 'function'
        ? __nickelHandlerBinding(props.onSubmit) : null;
    const node = {kind};
    if (props?.key !== undefined) node.key = props.key;
    for (const [name, value] of Object.entries(props || {}))
        if (value !== undefined && __nickelNativeProps.has(name)) node[name] = value;
    for (const [name, value] of Object.entries({action,contextAction,dragAction,dropAction,focusAction,blurAction,
        selectAction,moveAction,fileAction,closeAction,escapeAction,submitAction}))
        if (value !== null) node[name] = value;
    node.children = children.flat(Infinity).filter(child => child !== null && child !== false);
    return __nickelVirtualNativeNode(node);
}

function __nickelRollbackRender() {
    if (__pendingRender !== null) {
        const {handlers, previousHandlers, handlerSlots, hooks, records, values, effectsLength, dirty} = __pendingRender;
        __handlers = handlers;
        __previousHandlers = previousHandlers;
        __handlerSlots = handlerSlots;
        __nickelRestoreHooks(hooks, values, effectsLength);
        __componentRecords = records;
        __dirtyComponents = dirty;
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
    const {handlers, previousHandlers, hooks, records, values, effectsLength, effects, dirty} = __pendingEvent;
    __handlers = handlers;
    __previousHandlers = previousHandlers;
    __nickelRestoreHooks(hooks, values, effectsLength);
    __componentRecords = records;
    __effects = effects;
    __dirtyComponents = dirty;
    __pendingEvent = null;
}

function __nickelCommitRender() {
    const pending = __pendingRender;
    __pendingRender = null;
    if (pending === null) return;
    // Native subtrees are retained by their owning component records. Keeping
    // a second complete tree here would force every composition checkpoint to
    // clone the whole mount even when a single leaf is dirty.
    __acceptedNativeTree = null;
    __dirtyComponents.clear();
    const commitStarted=Date.now(),profile=__nickelProfile();
    for (const entry of pending.reducerEntries) {
        entry.reducer = entry.nextReducer;
        delete entry.nextReducer;
    }
    for (const entry of pending.removedEffects) __nickelRunCleanup(entry);
    for (const effect of pending.passiveEffects) {
        __nickelRunCleanup(effect.entry);
        effect.entry.deps = effect.deps;
        try {
            __runtimeCounters.effectsRun++;
            const cleanup = effect.setup();
            if (cleanup !== undefined && typeof cleanup !== 'function') continue;
            effect.entry.cleanup = cleanup;
        } catch (error) { __nickelCaptureFailure(effect.entry.owner, error, 'effect'); }
    }
    profile.lifecycleCommits++;profile.lifecycleCommitMillis+=Math.max(0,Date.now()-commitStarted);
    if(pending.passiveEffects.length&&__dirtyComponents.size)profile.consecutiveEffectTurns++;
    else profile.consecutiveEffectTurns=0;
    if(profile.consecutiveEffectTurns===25)
        __nickelDeveloperDiagnostic('effect-loop','effects scheduled another render for 25 consecutive commits',
            'Add stable dependencies and avoid unconditional state updates from an effect.',null,
            {turns:profile.consecutiveEffectTurns});
}

function __nickelAcceptEvent() {
    __pendingEvent = null;
}

function __nickelActiveEntry() {
    return __surfaceApps.get(__activeSurface) || App;
}

function __nickelRender(component = __nickelActiveEntry(), patchOnly = false) {
    if (__pendingRender !== null) throw Error('previous render was not finalized');
    __runtimeCounters.renders++;
    const profile=__nickelProfile(),reconciliationStarted=Date.now();profile.renders++;
    const previousHandlers = __handlers;
    const olderHandlers = __previousHandlers;
    const previousHooks = new Map(Array.from(__componentHooks, ([path, hooks]) => [path, hooks.slice()]));
    const previousValues = Array.from(__componentHooks.values(), hooks => hooks.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    const previousRecords = __componentRecords;
    __pendingRender = {handlers: previousHandlers, previousHandlers: olderHandlers, handlerSlots:new Map(__handlerSlots), hooks: previousHooks,
        records:previousRecords, values: previousValues, effectsLength: __effects.length, dirty:new Set(__dirtyComponents),
        passiveEffects: [], removedEffects: [], reducerEntries: []};
    __componentRecords = new Map(__componentRecords);
    __incrementalRender = __dirtyComponents.size > 0;
    __patchOnlyRender = patchOnly;
    __handlers = patchOnly ? previousHandlers.slice() : new __NickelHandlers();
    __previousHandlers = previousHandlers;
    if (!patchOnly) __handlerSlots = new Map();
    __listKeyErrors = [];
    __visitedComponents = new Set();
    __componentChildren = new Map();
    __retainedComponentChildren = __nickelIndexRetainedComponentChildren(previousRecords);
    __currentComponent = null;
    __hookIndex = 0;
    try {
        const virtual = __nickelResolveVirtual(h(component, {}));
        let result;
        if (patchOnly) {
            const patchStarted=Date.now();
            try { result=__nickelDirtyNativePatch(__pendingRender.records); }
            finally { profile.patchGenerationMillis+=Math.max(0,Date.now()-patchStarted); }
        } else {
            const node = __nickelMaterializeVirtual(virtual);
            __nickelAttachNativeRecords();
            __pendingRender.candidateNode = node;
            result=node;
        }
        const root = patchOnly ? virtual : result;
        if (root?.kind === 'window' && __listKeyErrors.length) throw Error(__listKeyErrors[0]);
        for (const path of __componentHooks.keys()) {
            if (!__visitedComponents.has(path)) {
                for (const entry of __componentHooks.get(path))
                    if (entry?.kind === 'effect') __pendingRender.removedEffects.push(entry);
                __componentHooks.delete(path);
                __componentRecords.delete(path);
            }
        }
        return patchOnly ? result : JSON.stringify(result);
    } catch (error) {
        __nickelRollbackRender();
        throw error;
    } finally {
        __patchOnlyRender = false;
        profile.reconciliationMillis+=Math.max(0,Date.now()-reconciliationStarted);
    }
}

function __nickelDirtyNativePatch(previousRecords) {
    const dirty = Array.from(__dirtyComponents).sort((left,right)=>left.length-right.length);
    const boundaries = [];
    for (const owner of dirty) {
        let selected = owner;
        while (selected !== null && !previousRecords.get(selected)?.native) {
            const identity=selected.lastIndexOf('/');
            const component=identity<0?-1:selected.lastIndexOf('/',identity-1);
            selected=component<0?null:selected.slice(0,component);
            if (selected==='root') selected=null;
        }
        if (selected === null || boundaries.some(path => selected === path || selected.startsWith(`${path}/`))) continue;
        boundaries.push(selected);
    }
    if (!boundaries.length) throw Error('dirty component has no admitted native ownership boundary');
    const operations=[];
    let nodesVisited=0;
    for (const path of boundaries) {
        const previous=previousRecords.get(path), current=__componentRecords.get(path);
        if (!previous?.native || !previous.nativePath || !current)
            throw Error('dirty native ownership boundary disappeared');
        let hostRoot;
        for (let ancestor = path; ancestor; ) {
            const record = previousRecords.get(ancestor);
            if (record?.nativePath === 'root' && Array.isArray(record.native)) {
                hostRoot = record.native.find(root => root?.kind === 'window'
                    && root.id === __surfaceStore.snapshot.id)?.__nativeId;
                break;
            }
            const identity = ancestor.lastIndexOf('/');
            const component = identity < 0 ? -1 : ancestor.lastIndexOf('/', identity - 1);
            ancestor = component < 0 ? null : ancestor.slice(0, component);
        }
        if (hostRoot && previous.nativePath !== 'root'
            && previous.nativePath !== hostRoot
            && !previous.nativePath.startsWith(`${hostRoot}/`)) {
            // Keep the last admitted boundary so another hidden update does
            // not escalate to materializing the complete fragment.
            __componentRecords.set(path, {...current,native:previous.native,
                nativePath:previous.nativePath});
            __nickelRefreshNativeAncestors(previousRecords,path,
                previous.nativePath,previous.native);
            continue;
        }
        const next=__nickelMaterializeVirtual(current.output,previous.nativePath);
        __nickelRetireBoundaryHandlers(previous.native,next);
        const patch=__nickelNativePatch(previous.native,next);
        operations.push(...patch.operations);nodesVisited+=patch.counters.nodesVisited;
        __nickelRefreshNativeAncestors(previousRecords,path,previous.nativePath,next);
    }
    __nickelAttachNativeRecords(boundaries);
    const profile=__nickelProfile();profile.patches++;profile.patchOperations+=operations.length;
    profile.patchNodesVisited+=nodesVisited;
    return {version:1,operations,counters:{nodesVisited,nodesMutated:operations.length,
        localMaterializations:0,expansionNodes:0,treeBytes:0}};
}

// A leaf patch also changes the accepted native snapshot held by every
// component boundary above it. Refresh those snapshots with path-copying so a
// later ancestor update still has an exact, rollback-safe comparison base.
function __nickelRetireBoundaryHandlers(previous,next) {
    const live=new Set();
    function visit(value,slot) {
        if(!value||typeof value!=='object')return;
        if(Array.isArray(value)) {for(const child of value)visit(child,slot);return;}
        for(const key of Object.values(value.__handlerSlots??{}))slot(key);
        for(const child of value.children??[])visit(child,slot);
    }
    visit(next,slot=>live.add(slot));
    visit(previous,slot=>{
        if(!live.has(slot)) {
            const action=__handlerSlots.get(slot);
            __handlerSlots.delete(slot);__handlers.delete(action);
        }
    });
}

function __nickelRefreshNativeAncestors(previousRecords, boundary, target, replacement) {
    function replace(value) {
        if (!value || typeof value !== 'object') return [false,value];
        if (!Array.isArray(value) && value.__nativeId === target) return [true,replacement];
        if (!Array.isArray(value) && value.__nativeId
            && !target.startsWith(`${value.__nativeId}/`)) return [false,value];
        const children=Array.isArray(value)?value:value.children;
        if (!Array.isArray(children)) return [false,value];
        for(let index=0;index<children.length;index++) {
            const [changed,child]=replace(children[index]);
            if(!changed)continue;
            const nextChildren=children.slice();nextChildren[index]=child;
            return Array.isArray(value)?[true,nextChildren]:[true,{...value,children:nextChildren}];
        }
        return [false,value];
    }
    for(const [path,record] of __componentRecords) {
        if(path!==boundary&&!boundary.startsWith(`${path}/`))continue;
        const previous=previousRecords.get(path);
        const base=record.native??previous?.native;
        if(!base)continue;
        const [changed,native]=replace(base);
        if(changed)__componentRecords.set(path,{...record,native,nativePath:record.nativePath??previous?.nativePath});
    }
}

function __nickelNativeEqual(left,right) {
    if (Object.is(left,right)) return true;
    if (!left || !right || typeof left!=='object' || typeof right!=='object') return false;
    if (Array.isArray(left)!==Array.isArray(right)) return false;
    const leftKeys=Object.keys(left),rightKeys=Object.keys(right);
    return leftKeys.length===rightKeys.length && leftKeys.every(key =>
        Object.prototype.hasOwnProperty.call(right,key) && __nickelNativeEqual(left[key],right[key]));
}

// Produce a bounded transport delta against the last host-accepted native
// tree. A structural children change replaces the containing native node;
// otherwise the walk descends and ships only changed native subtrees.
function __nickelNativePatch(previous, next) {
    const operations = [];
    let visited = 0;
    function primitive(value) {
        return value === null || ['string','number','boolean','undefined'].includes(typeof value);
    }
    function emitProperties(left, right) {
        const keys = new Set([...Object.keys(left), ...Object.keys(right)]);
        keys.delete('children');
        keys.delete('__nativeId');
        keys.delete('__handlerSlots');
        keys.delete('kind');
        keys.delete('key');
        for (const key of keys) {
            if (__nickelNativeEqual(left[key],right[key])) continue;
            const slot = right.__handlerSlots?.[key] ?? left.__handlerSlots?.[key];
            if (slot && Number.isSafeInteger(left[key]) && left[key] >= 0
                    && Number.isSafeInteger(right[key]) && right[key] >= 0) {
                operations.push({op:'replaceHandlerSlot', slot, action:right[key]});
            } else if (slot) {
                // Activating or removing a conditional callback changes the
                // native handler-slot topology, so install the containing node
                // atomically instead of addressing authority that does not yet
                // exist (or has just disappeared).
                return false;
            } else if (primitive(left[key]) && primitive(right[key])) {
                operations.push({op:'setPrimitive', target:right.__nativeId, property:key,
                    value:right[key] === undefined ? null : right[key]});
            } else return false;
        }
        return true;
    }
    function keyed(children) {
        if (!children.length) return true;
        const keys = new Set(), ids = new Set();
        for (const child of children) {
            if (!child || typeof child !== 'object' || Array.isArray(child)
                || child.key === undefined || !child.__nativeId) return false;
            const key = String(child.key);
            if (keys.has(key) || ids.has(child.__nativeId)) return false;
            keys.add(key); ids.add(child.__nativeId);
        }
        return true;
    }
    function emitKeyedChildren(parent, before, after) {
        if (!keyed(before) || !keyed(after)) return false;
        const desired = new Map(after.map(child => [child.__nativeId, child]));
        const working = before.slice();
        for (let index = working.length - 1; index >= 0; index--) {
            const child = working[index];
            if (!desired.has(child.__nativeId)) {
                operations.push({op:'removeChild', parent:parent.__nativeId,
                    key:String(child.key), child_id:child.__nativeId, index});
                working.splice(index, 1);
            }
        }
        for (let index = 0; index < after.length; index++) {
            const wanted = after[index];
            if (working[index]?.__nativeId === wanted.__nativeId) continue;
            const from = working.findIndex(child => child.__nativeId === wanted.__nativeId);
            if (from < 0) {
                operations.push({op:'insertChild', parent:parent.__nativeId,
                    key:String(wanted.key), child_id:wanted.__nativeId, index, node:wanted});
                working.splice(index, 0, wanted);
            } else {
                operations.push({op:'moveChild', parent:parent.__nativeId,
                    key:String(wanted.key), child_id:wanted.__nativeId, from, to:index});
                working.splice(index, 0, working.splice(from, 1)[0]);
            }
        }
        const oldById = new Map(before.map(child => [child.__nativeId, child]));
        for (const child of after) {
            const old = oldById.get(child.__nativeId);
            if (old && !__nickelNativeEqual(old,child)) walk(old, child);
        }
        return true;
    }
    function walk(left, right) {
        visited++;
        if (Array.isArray(left) && Array.isArray(right)) {
            // A package fragment is represented by a separate native host for
            // each window. Its component owns the array, not a synthetic node.
            // Keep the root topology stable and patch only this host's window.
            const id = __surfaceStore.snapshot.id;
            if (!id || left.length !== right.length || left.some((root,index) =>
                root?.kind !== right[index]?.kind || root?.id !== right[index]?.id
                || root?.__nativeId !== right[index]?.__nativeId))
                throw Error('native fragment patch requires stable window topology');
            const before = left.filter(root => root?.kind === 'window' && root.id === id);
            const after = right.filter(root => root?.kind === 'window' && root.id === id);
            if (before.length !== 1 || after.length !== 1)
                throw Error('native fragment patch has no unique host window');
            walk(before[0], after[0]);
            return;
        }
        if (!left || !right || typeof left !== 'object' || typeof right !== 'object'
            || Array.isArray(left) || Array.isArray(right)
            || left.__nativeId !== right.__nativeId || left.kind !== right.kind) {
            if (right?.__nativeId) operations.push({op:'replaceSubtree', target:right.__nativeId, node:right});
            return;
        }
        const operationStart = operations.length;
        if (!emitProperties(left, right)) {
            operations.length = operationStart;
            operations.push({op:'replaceSubtree', target:right.__nativeId, node:right});
            return;
        }
        const before = left.children ?? [];
        const after = right.children ?? [];
        if (!Array.isArray(before) || !Array.isArray(after)) {
            operations.length = operationStart;
            operations.push({op:'replaceSubtree', target:right.__nativeId, node:right});
            return;
        }
        if (before.every(primitive) && after.every(primitive)) {
            if (!__nickelNativeEqual(before,after))
                operations.push({op:'setPrimitive', target:right.__nativeId, property:'children', value:after});
            return;
        }
        if ((before.length !== after.length
            || before.some((child, index) => child?.__nativeId !== after[index]?.__nativeId))
            && emitKeyedChildren(right, before, after)) return;
        if (before.length !== after.length) {
            operations.length = operationStart;
            operations.push({op:'replaceSubtree', target:right.__nativeId, node:right});
            return;
        }
        for (let index = 0; index < after.length; index++) {
            const a = before[index], b = after[index];
            if (__nickelNativeEqual(a,b)) continue;
            if (a?.__nativeId && b?.__nativeId && a.__nativeId === b.__nativeId) walk(a, b);
            else {
                operations.length = operationStart;
                operations.push({op:'replaceSubtree', target:right.__nativeId, node:right});
                return;
            }
        }
    }
    if (previous === null) throw Error('native patch has no accepted base');
    walk(previous, next);
    __nickelRecordNativeMutations(operations,visited);
    return {version:1, operations, counters:{nodesVisited:visited, nodesMutated:operations.length}};
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
    __pendingEvent = {handlers: __handlers, previousHandlers: __previousHandlers, hooks,
        records:__componentRecords, values,
        effectsLength, effects: __effects.slice(), dirty:new Set(__dirtyComponents)};
    try {
        for (const [action, value] of events) {
            const handler = (previous ? __previousHandlers : __handlers).get(action);
            if (handler) handler(value);
        }
        return __nickelRender();
    } catch (error) {
        __nickelRollbackEvent();
        throw error;
    }
}

// Structured scheduler entry point. It deliberately still produces a complete
// tree when dirty; the native mutation protocol can replace that payload later.
function __nickelDispatchBatchScheduled(events, previous = false) {
    const hooks = new Map(Array.from(__componentHooks, ([path, slots]) => [path, slots.slice()]));
    const values = Array.from(__componentHooks.values(), slots => slots.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    const effectsLength = __effects.length;
    __pendingEvent = {handlers:__handlers, previousHandlers:__previousHandlers, hooks,
        records:__componentRecords, values,
        effectsLength, effects:__effects.slice(), dirty:new Set(__dirtyComponents)};
    try {
        for (const [action, value] of events) {
            const handler = (previous ? __previousHandlers : __handlers).get(action);
            if (handler) handler(value);
        }
        const dirty = Array.from(__dirtyComponents);
        if (!dirty.length) return JSON.stringify({rendered:false, dirty});
        return JSON.stringify({rendered:true, dirty, node:JSON.parse(__nickelRender())});
    } catch (error) {
        __nickelRollbackEvent();
        throw error;
    }
}

function __nickelDispatchBatchPatched(events, previous = false) {
    const hooks = new Map(Array.from(__componentHooks, ([path, slots]) => [path, slots.slice()]));
    const values = Array.from(__componentHooks.values(), slots => slots.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    const effectsLength = __effects.length;
    __pendingEvent = {handlers:__handlers, previousHandlers:__previousHandlers, hooks,
        records:__componentRecords, values,
        effectsLength, effects:__effects.slice(), dirty:new Set(__dirtyComponents)};
    try {
        for (const [action, value] of events) {
            const handler = (previous ? __previousHandlers : __handlers).get(action);
            if (handler) handler(value);
        }
        const dirty = Array.from(__dirtyComponents);
        if (!dirty.length) return JSON.stringify({rendered:false, dirty});
        const patch = __nickelRender(undefined,true);
        return JSON.stringify({rendered:true, dirty, patch});
    } catch (error) {
        __nickelRollbackEvent();
        throw error;
    }
}

function __nickelDispatchSlotsPatched(events, previous = false) {
    return __nickelDispatchBatchPatched(events.map(([slot, value]) => {
        const action = __handlerSlots.get(slot);
        if (action === undefined) throw Error(`retired component event slot: ${slot}`);
        return [action, value];
    }), previous);
}

function __nickelReconciliationRequest() {
    return JSON.stringify({requested:__dirtyComponents.size > 0, dirty:Array.from(__dirtyComponents)});
}
function __nickelConsumeReconciliation() { __dirtyComponents.clear(); }

// Native composition checkpoints cover bootstrap-owned presentation state.
// They deliberately do not snapshot package globals or closure-captured values.
// Trusted host extensions may participate in rollback without engine-owned domain schemas.
const __hostCheckpointParticipants = [];
function __twinkleRegisterCheckpointParticipant(participant) {
    if (__compositionCheckpoint !== null || __pendingRender !== null || __pendingEvent !== null)
        throw Error('checkpoint registration requires an idle runtime');
    if (!participant || typeof participant.capture !== 'function' || typeof participant.restore !== 'function'
        || __hostCheckpointParticipants.length >= 16) throw Error('invalid checkpoint participant');
    __hostCheckpointParticipants.push(Object.freeze({...participant}));
}
let __compositionCheckpoint = null;
function __nickelPlatformMaintenanceReady() {
    return __compositionCheckpoint === null && __pendingRender === null && __pendingEvent === null;
}
function __nickelSurfaceWorkPending(id) {
    if (!__nickelPlatformMaintenanceReady()) throw Error('surface work query requires an idle runtime');
    const state = id === __activeSurface
        ? {dirty:__dirtyComponents, effects:__effects}
        : __surfaceStates.get(id);
    return !!state && (state.dirty.size > 0 || state.effects.length > 0);
}
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
    function state(hooks, records, handlers, previousHandlers, handlerSlots, effects, data, dirty, surfaceStore, acceptedNativeTree) {
        return {hooks:new Map(Array.from(hooks, ([path, slots]) => [path, slots.slice()])),
            records:new Map(records),
            values:Array.from(hooks.values(), slots => slots.map(entry => copy(entry.kind === 'ref' ? entry.value.current : entry.value))),
            handlers:handlers.slice(), previousHandlers:previousHandlers.slice(), handlerSlots:new Map(handlerSlots), effects:copy(effects), data,
            dirty:new Set(dirty), surfaceStore, acceptedNativeTree:copy(acceptedNativeTree)};
    }
    const active = state(__componentHooks, __componentRecords, __handlers, __previousHandlers, __handlerSlots, __effects, __nickelData, __dirtyComponents, __surfaceStore, __acceptedNativeTree);
    const surfaces = new Map(Array.from(__surfaceStates, ([id, value]) => [id,
        state(value.hooks, value.records, value.handlers, value.previousHandlers, value.handlerSlots ?? new Map(), value.effects, value.data, value.dirty, value.surfaceStore, value.acceptedNativeTree)]));
    __compositionCheckpoint = {active, surfaces, graph:seen, extensible, apps:new Map(__surfaceApps), activeSurface:__activeSurface,
        extensions:__hostCheckpointParticipants.map(participant=>participant.capture(copy))};
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
        return {hooks:state.hooks, records:state.records, handlers:state.handlers, previousHandlers:state.previousHandlers, handlerSlots:state.handlerSlots,
            effects:original(state.effects), data:state.data, dirty:state.dirty,
            surfaceStore:state.surfaceStore, acceptedNativeTree:original(state.acceptedNativeTree)};
    }
    __surfaceStates.clear();
    for (const [id, state] of checkpoint.surfaces) __surfaceStates.set(id, restore(state));
    __surfaceApps.clear();
    for (const [id, app] of checkpoint.apps) __surfaceApps.set(id, app);
    const active = restore(checkpoint.active);
    __componentHooks = active.hooks; __componentRecords = active.records; __handlers = active.handlers; __previousHandlers = active.previousHandlers;
    __handlerSlots = active.handlerSlots;
    __effects = active.effects; __nickelData = active.data; __activeSurface = checkpoint.activeSurface;
    __dirtyComponents = active.dirty;
    __surfaceStore = active.surfaceStore;
    __acceptedNativeTree = active.acceptedNativeTree;
    __hostCheckpointParticipants.forEach((participant,index)=>participant.restore(checkpoint.extensions[index],original));
    __visitedComponents = new Set(); __componentChildren = new Map(); __currentComponent = null; __hookIndex = 0; __listKeyErrors = [];
}
