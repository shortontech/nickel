// Nickel's global JSX API. This file is for editors and the development
// compiler; the installed plugin still contains plain JavaScript. Components
// and public component references are callable in JSX. Children are ordinary JSX values.
declare namespace JSX {
    interface Element {}
    interface ElementChildrenAttribute { children: {} }
    interface IntrinsicElements {
        div: NickelDivProps;
    }
}

type NickelChild = JSX.Element | string | number | null | undefined | boolean | ReadonlyArray<NickelChild>;
type NickelComponent<P = NickelProps> = (props: P) => JSX.Element | null;
type NickelAnchor = "center" | "top-left" | "top-center" | "top-right" | "bottom-left" | "bottom-center" | "bottom-right";
type NickelJson = null | boolean | number | string | ReadonlyArray<NickelJson> | { readonly [key: string]: NickelJson };
type NickelColor = number; // 0xAARRGGBB
type NickelClick = () => void;
interface NickelDragGesture {
    phase: "start" | "move" | "end" | "cancel";
    x: number;
    y: number;
    bounds: { x: number; y: number; width: number; height: number };
}
interface NickelDropGesture {
    x: number;
    y: number;
    sourceId: string;
    sourceBounds: { x: number; y: number; width: number; height: number };
    targetId: string;
    targetBounds: { x: number; y: number; width: number; height: number };
}

interface NickelProps {
    key?: string | number;
    className?: string;
    children?: NickelChild;
}

interface NickelDivProps extends NickelProps {
    id?: string;
    onClick?: NickelClick;
    onDrop?: (gesture: NickelDropGesture) => void;
    role?: "button" | "radio" | "radiogroup" | "option" | "group";
    "aria-label"?: string;
    "aria-checked"?: boolean;
    "aria-selected"?: boolean;
    disabled?: boolean;
}

interface NickelPanelProps extends NickelProps {
    background?: NickelColor;
    height?: number;
}
interface NickelSurfaceProps extends NickelProps {
    id?: string;
    width: number;
    height: number;
    background?: NickelColor;
}
interface NickelWindowProps extends NickelProps {
    /** Native window activation, distinct from control focus; duplicate reports are ignored. */
    onFocus?: () => void;
    /** Native focus loss after activation; initial unactivated focus loss is ignored. */
    onBlur?: () => void;
    onSubmit?: () => void;
    onEscape?: () => void;
    id?: string;
    title?: string;
    width: number | "100%";
    height: number | "100%";
    placement?: "managed" | "fixed";
    output?: "primary" | "all";
    edge?: "top" | "bottom" | "left" | "right";
    anchor?: "center" | "top-left" | "top-center" | "top-right" | "bottom-left" | "bottom-center" | "bottom-right";
    reserveWorkArea?: boolean;
    bottomOffset?: number;
    background?: NickelColor;
}
interface NickelViewportProps extends NickelProps {
    background?: NickelColor;
    /** Inset from every edge, from 0 to 256 logical pixels. */
    padding?: number;
}
interface NickelBoxProps extends NickelProps {
    x: number;
    y: number;
    width: number;
    height: number;
    background?: NickelColor;
    radius?: number;
}
interface NickelTextProps extends NickelProps { color?: NickelColor; wrap?: boolean; maxLines?: number }
interface NickelButtonProps extends NickelProps {
    id?: string;
    width?: number;
    height?: number;
    accessibilityLabel?: string;
    icon?: string;
    showLabel?: boolean;
    disabled?: boolean;
    state?: "selected" | "unselected" | "disabled";
    onClick: NickelClick;
    onFocus?: NickelClick;
    onBlur?: NickelClick;
    onContextMenu?: NickelClick;
    onDrag?: (gesture: NickelDragGesture) => void;
    onDrop?: (gesture: NickelDropGesture) => void;
}
interface NickelTextFieldProps extends NickelProps {
    id?: string;
    value?: string;
    accessibilityLabel?: string;
    disabled?: boolean;
    placeholder?: string;
    /** Mask the displayed value and protect the surface from remote inspection. */
    secure?: boolean;
    onChange: (value: string) => void;
    onFocus?: NickelClick;
    onBlur?: NickelClick;
}
interface NickelSliderProps extends NickelProps {
    id?: string;
    value: number;
    accessibilityLabel: string;
    /** Defaults to 0..1; onChange receives a value in this numeric range. */
    min?: number;
    max?: number;
    step?: number;
    onChange: (value: number) => void;
}
interface NickelSwitchProps extends NickelProps {
    id: string;
    state: "on" | "off" | "disabled-on" | "disabled-off";
    accessibilityLabel: string;
    onClick?: NickelClick;
}
interface NickelColorSwatchProps extends NickelProps {
    id?: string;
    /** CSS color; omit for the custom-color button. */
    color?: string;
    selected?: boolean;
    accessibilityLabel: string;
    onClick: NickelClick;
}
interface NickelSelectProps extends NickelProps {
    id: string;
    value: string;
    open?: boolean;
    accessibilityLabel: string;
    onClick: NickelClick;
}
interface NickelOptionProps extends NickelProps {
    id: string;
    onClick: NickelClick;
}
interface NickelImageProps extends NickelProps {
    asset: string;
    width: number;
    height: number;
    fit?: "contain" | "cover" | "stretch";
}
interface NickelImageButtonProps extends NickelImageProps {
    id?: string;
    accessibilityLabel: string;
    onClick: NickelClick;
    onContextMenu?: NickelClick;
}
interface NickelDialogProps extends NickelProps {
    id?: string;
    anchor: string;
    open?: boolean;
    onClose?: NickelClick;
    width?: number;
    height?: number;
}
interface NickelMenuProps extends NickelProps {
    id: string;
    anchor: string;
    open?: boolean;
    x?: number;
    y?: number;
}
interface NickelMenuItemProps extends NickelProps {
    id: string;
    label?: string;
    onClick?: NickelClick;
    disabledReason?: string;
    shortcut?: string;
    separatorBefore?: boolean;
}
interface NickelBadgeProps extends NickelProps {
    item?: string;
    label?: string;
    count: number;
    color?: NickelColor;
}
interface NickelProgressProps extends NickelProps {
    percent: number;
    width: number;
    height: number;
}
declare function Fragment(props:NickelProps):JSX.Element;
declare function h(kind: unknown, props?: object | null, ...children: NickelChild[]): JSX.Element;
/** @deprecated Compatibility helper; use Window or FixedWindow for new surfaces. */
declare function Panel(props: NickelPanelProps): JSX.Element;
declare function Surface(props: NickelSurfaceProps): JSX.Element;
declare function Window(props: NickelWindowProps): JSX.Element;
/** Convenience JSX component that renders Window with fixed placement. */
declare function FixedWindow(props: Omit<NickelWindowProps, "placement">): JSX.Element;
declare function Viewport(props: NickelViewportProps): JSX.Element;
declare function Box(props: NickelBoxProps): JSX.Element;
/** Positions Box children by their x and y coordinates. Size it with CSS. */
declare function Layer(props: NickelProps & { id?: string }): JSX.Element;
/** Generic CSS layout box. Defaults to block layout. */
declare function Div(props: NickelDivProps): JSX.Element;
declare function Badge(props: NickelBadgeProps): JSX.Element;
declare function Row(props: NickelProps): JSX.Element;
declare function Column(props: NickelProps): JSX.Element;
declare function ScrollView(props: NickelProps & { id: string; height?: number; grow?: boolean }): JSX.Element;
declare function Text(props: NickelTextProps): JSX.Element;
declare function Image(props: NickelImageProps): JSX.Element;
declare function ImageButton(props: NickelImageButtonProps): JSX.Element;
declare function Progress(props: NickelProgressProps): JSX.Element;
declare function TextField(props: NickelTextFieldProps): JSX.Element;
declare function Checkbox(props: NickelProps & { id?: string; checked?: boolean; indeterminate?: boolean; disabled?: boolean; label?: string; accessibilityLabel?: string; onChange?: (checked: boolean) => void; onClick?: NickelClick }): JSX.Element;
declare function Slider(props: NickelSliderProps): JSX.Element;
declare function Switch(props: NickelSwitchProps): JSX.Element;
declare function ColorSwatch(props: NickelColorSwatchProps): JSX.Element;
declare function Select(props: NickelSelectProps): JSX.Element;
declare function Option(props: NickelOptionProps): JSX.Element;
declare function Button(props: NickelButtonProps): JSX.Element;
declare function Spacer(props: NickelProps): JSX.Element;
/** Host-provided content placed inside this component's layout box. */
declare function Slot(props: NickelProps & { id: string }): JSX.Element;
declare function Dialog(props: NickelDialogProps): JSX.Element;
declare function Menu(props: NickelMenuProps): JSX.Element;
declare function MenuItem(props: NickelMenuItemProps): JSX.Element;

declare function useState<T>(initial: T | (() => T)): [T, (next: T | ((previous: T) => T)) => void];
declare function useReducer<S, A>(reducer: (state: S, action: A) => S, initialState: S): [S, (action: A) => void];
declare function useReducer<S, A, I>(reducer: (state: S, action: A) => S, initialArg: I, init: (initialArg: I) => S): [S, (action: A) => void];
declare function useRef<T>(initial: T): { current: T };
/** Stable, opaque ID for accessibility relationships within this component's mounted surface. */
declare function useId(): string;
interface NickelContext<T> { readonly Provider: NickelComponent<{value:T;children?:NickelChild}>; readonly defaultValue: T }
declare function createContext<T>(defaultValue: T): NickelContext<T>;
declare function useContext<T>(context: NickelContext<T>): T;
declare function useWindows(): ReadonlyArray<Readonly<NickelNativeWindow>>;
declare function useWindows<T>(selector: (windows: ReadonlyArray<Readonly<NickelNativeWindow>>) => T): T;
declare function useActiveWindow(): Readonly<NickelNativeWindow> | null;
declare function useApplications(): ReadonlyArray<Readonly<NickelApplication>>;
declare function useApplications<T>(selector: (applications: ReadonlyArray<Readonly<NickelApplication>>) => T): T;
declare function useNotifications(): Readonly<NickelNotificationSnapshot>;
declare function useNotifications<T>(selector: (notifications: Readonly<NickelNotificationSnapshot>) => T): T;
declare function useWorkspaces(): Readonly<NickelWorkspaceSnapshot>;
declare function useWorkspaces<T>(selector: (workspaces: Readonly<NickelWorkspaceSnapshot>) => T): T;
declare function useWorkspace(): Readonly<{id:string;active:boolean}>|null;
interface NickelOutputsSnapshot extends NickelAvailability {readonly generation:number;readonly revision:string|null;readonly outputs:NickelDisplaySnapshot["outputs"]}
declare function useOutputs(): Readonly<NickelOutputsSnapshot>;
declare function useOutputs<T>(selector:(outputs:Readonly<NickelOutputsSnapshot>)=>T):T;
interface NickelLocaleSnapshot {readonly generation:number;readonly tag:string;readonly direction:"ltr"|"rtl";readonly known:boolean}
declare function useLocale():Readonly<NickelLocaleSnapshot>;
interface NickelExternalStore<T> {readonly subscribe:(listener:()=>void)=>()=>void;readonly getSnapshot:()=>T}
declare const NickelStores:Readonly<{
    windows:NickelExternalStore<ReadonlyArray<Readonly<NickelNativeWindow>>>;
    applications:NickelExternalStore<ReadonlyArray<Readonly<NickelApplication>>>;
    notifications:NickelExternalStore<Readonly<NickelNotificationSnapshot>>;
    workspaces:NickelExternalStore<Readonly<NickelWorkspaceSnapshot>>;
    outputs:NickelExternalStore<Readonly<NickelOutputsSnapshot>>;
    locale:NickelExternalStore<Readonly<NickelLocaleSnapshot>>;
    theme:NickelExternalStore<Readonly<NickelThemeSnapshot>>;
    capabilities:NickelExternalStore<Readonly<Record<NickelCapability,Readonly<NickelCapabilitySnapshot>>>>;
}>;
/** Only matching functions from a host-branded NickelStores entry are accepted. */
declare function useSyncExternalStore<T>(subscribe:NickelExternalStore<T>["subscribe"],getSnapshot:NickelExternalStore<T>["getSnapshot"]):T;
declare function memo<P>(component:NickelComponent<P>,compare?:(previous:Readonly<P & {children?:NickelChild}>,next:Readonly<P & {children?:NickelChild}>)=>boolean):NickelComponent<P>;
interface NickelThemePalette {
    readonly background:NickelColor; readonly panel:NickelColor; readonly surface:NickelColor;
    readonly surfaceHover:NickelColor; readonly text:NickelColor; readonly muted:NickelColor;
    readonly accent:NickelColor; readonly accentSoft:NickelColor; readonly complement:NickelColor;
}
interface NickelThemeSnapshot {
    readonly generation:number;
    readonly mode:"light"|"dark"|"unknown";
    readonly accent:NickelColor|null;
    readonly accentHue:number|null;
    readonly accentIntensity:number|null;
    /** null means the host has not bridged an authoritative preference. */
    readonly reducedMotion:boolean|null;
    /** null means the host has not bridged an authoritative preference. */
    readonly reducedTransparency:boolean|null;
    readonly palette:Readonly<NickelThemePalette>|null;
}
declare function useTheme(): Readonly<NickelThemeSnapshot>;
declare function useTheme<T>(selector: (theme: Readonly<NickelThemeSnapshot>) => T): T;
declare function useReducedMotion(): boolean|null;
type NickelCapability =
    | "launcher-show" | "control-center-show" | "on-screen-keyboard-show"
    | "on-screen-keyboard-read" | "on-screen-keyboard-input"
    | "applications-read" | "applications-launch" | "applications-pin"
    | "associations-read" | "associations-control" | "plugins-read" | "plugins-control"
    | "features-read" | "features-control" | "shortcuts-read"
    | "preferences-read" | "preferences-control"
    | "windows-read" | "windows-focus" | "windows-context"
    | "desktop-read" | "desktop-arrange" | "desktop-files-open" | "desktop-files-manage"
    | "tray-read" | "tray-activate" | "tray-context"
    | "appearance-read" | "appearance-control" | "wallpaper-read" | "wallpaper-control"
    | "audio-read" | "audio-control" | "network-read" | "network-control"
    | "bluetooth-read" | "bluetooth-control" | "desktop-control" | "display-control"
    | "session-control" | "workspaces-read" | "workspaces-switch"
    | "notifications-read" | "notifications-act"
    | "settings-read" | "settings-write" | "settings-show" | "projects-menu-show"
    | "session-logout-request" | "run-command";
interface NickelCapabilitySnapshot {
    readonly declared:boolean;
    /** null means runtime availability has no authoritative observation. */
    readonly available:boolean|null;
    readonly reason:string|null;
}
declare function useCapability(capability: NickelCapability): Readonly<NickelCapabilitySnapshot>;
declare function useEffect(setup: () => void | (() => void), dependencies?: ReadonlyArray<unknown>): void;
interface NickelSurfaceSnapshot {
    readonly generation:number;
    /** Opaque host-owned identity for this exact mounted surface. */
    readonly mountId:string|null;
    readonly id:string|null;
    readonly kind:"panel"|"dock"|"desktop"|"window"|"dialog"|"overlay"|null;
    readonly logicalSize:Readonly<{width:number;height:number}>|null;
    /** Unavailable until the native output authority is projected to this mount. */
    readonly output:string|null;
    readonly availableSize:Readonly<{width:number;height:number}>|null;
    readonly scaleFactor:number|null;
    readonly focused:boolean|null;
    readonly visible:boolean|null;
}
declare function useSurface():NickelSurfaceSnapshot;
declare function useOutput():NickelDisplaySnapshot["outputs"][number]|null;
declare function useScaleFactor():number|null;
declare function useSurfaceFocus():boolean|null;
declare function useMemo<T>(factory: () => T, dependencies?: ReadonlyArray<unknown>): T;
declare function useCallback<T extends (...args: never[]) => unknown>(callback: T, dependencies?: ReadonlyArray<unknown>): T;

type NickelSurfaceRequest = Readonly<
    | { type: "show-plugin-surface"; surfaceId: string }
    | { type: "hide-plugin-surface"; surfaceId: string }
    | {
        type: "surface.setPlacement";
        surfaceId: string;
        anchor: "center" | "top-left" | "top-center" | "top-right" | "bottom-left" | "bottom-center" | "bottom-right";
        offsetX: number;
        offsetY: number;
    }
>;

/** The complete requested arrangement of connected outputs. */
interface NickelDisplayLayout {
    primary: string;
    placements: ReadonlyArray<Readonly<{
        name: string;
        x: number;
        y: number;
        enabled: boolean;
        /** Fractional scale units; 120 is 100%. */
        scale_120?: number;
        /** Omit to preserve the current native orientation. */
        transform?: NickelDisplayTransform;
        mode?: Readonly<{
            width: number;
            height: number;
            /** Vertical refresh rate in millihertz. */
            refresh_millihz: number;
        }> | null;
    }>>;
}

interface NickelDisplayMode {
    width: number;
    height: number;
    refresh_millihz: number;
}

type NickelDisplayTransform = "normal" | "rotate90" | "rotate180" | "rotate270" | "flipped" | "flipped90" | "flipped180" | "flipped270";
interface NickelDisplaySnapshot {
    projectionModes?: ReadonlyArray<Readonly<{id:NickelProjectionMode;label:string}>>;
    revision?: string;
    operations?: Readonly<{setOrientation?: boolean; setApplicationScale?: boolean; identify?: boolean}>;
    transforms?: ReadonlyArray<NickelDisplayTransform>;
    pending_confirmation?: boolean;
    can_confirm?: boolean;
    can_revert?: boolean;
    application_scale?: NickelApplicationScaleSnapshot;
    available: boolean;
    reason?: string;
    outputs: ReadonlyArray<Readonly<{
        name: string;
        model: string;
        geometry: Readonly<{ x: number; y: number; width: number; height: number }>;
        work_area: Readonly<{ x: number; y: number; width: number; height: number }>;
        scale_120: number;
        transform: NickelDisplayTransform;
        physical_width_mm: number;
        physical_height_mm: number;
        primary: boolean;
        enabled: boolean;
        modes: ReadonlyArray<Readonly<NickelDisplayMode>>;
        current_mode: Readonly<NickelDisplayMode> | null;
    }>>;
}

interface NickelPluginStatus {
    readonly id:string;
    readonly name:string;
    readonly author:string | null;
    readonly version:string | null;
    readonly enabled:boolean;
    readonly shell:boolean;
    readonly selected:boolean;
    readonly health:Readonly<{state:"disabled" | "idle" | "starting" | "running" | "failed";reason?:string}>;
    readonly grants:ReadonlyArray<string>;
    readonly surfaces:ReadonlyArray<string>;
    readonly composition:ReadonlyArray<string>;
    readonly settings:ReadonlyArray<{id:string;label:string;description:string;value:boolean|number|string;kind:Readonly<{kind:"boolean"|"integer"|"text"|"choice";min?:number;max?:number;max_length?:number;options?:ReadonlyArray<string>}>}>;
    readonly memory:Readonly<{jsHeapBytes:number | null;nativeUiBytes:number | null;textureBytes:number | null;trackedPeakBytes:number | null;timers:number;subscriptions:number;componentBreakdownAvailable:false}>;
}

/** Read clients return copies; missing grants produce empty or unavailable snapshots. */
interface NickelContribution {readonly id:string;readonly provider:string;readonly version:string;readonly key:string;readonly component:NickelComponent}
interface NickelWorkspaceSnapshot extends NickelAvailability {readonly generation:number;readonly writable:boolean;revision:string|null;workspaces:ReadonlyArray<Readonly<{id:string;active:boolean}>>;activeWorkspace:string|null;operations:Readonly<{switch:boolean;create:boolean;remove:boolean}>}
type NickelProjectionMode = "internal"|"duplicate"|"extend"|"external";
interface NickelClockSnapshot {unixMilliseconds:number;utcOffsetMinutes:number}
interface NickelAvailability { available: boolean; reason?: string | null }
interface NickelWritable extends NickelAvailability { writable?: boolean; revision?: string }
interface NickelAppearancePreferences {
    theme: "system" | "light" | "dark";
    /** Hue 0..359, or null to follow the native system accent. */
    accent_hue: number | null;
    /** Intensity 0..100, or null to follow the native system accent. */
    accent_intensity: number | null;
    reduce_transparency: boolean;
    animations: "off" | "reduced" | "normal";
}
interface NickelAppearanceSnapshot extends NickelAvailability {
    writable?: boolean; generation?: number; observed_at_us?: number;
    configured?: Readonly<NickelAppearancePreferences>;
    resolved?: Readonly<{theme:"light"|"dark"; hue:number; intensity:number; accent:NickelColor}>;
}
type NickelWallpaperPosition = "center" | "tile" | "stretch" | "fit" | "span" | "fill";
interface NickelWallpaperImage { id:string; configured:boolean; label?:string; previewAsset?:string }
interface NickelWallpaperSnapshot extends NickelAvailability {
    writable?: boolean; generation?:number; observed_at_us?:number;
    configured?:Readonly<{custom_image_configured:boolean; position:NickelWallpaperPosition}>;
    images?:ReadonlyArray<Readonly<NickelWallpaperImage>>;
    selected_image_decoded?:boolean; runtime_reload_requested?:boolean;
    chooser?:Readonly<{available:boolean; pending:boolean; result:Readonly<{status:string;detail?:string}>|null}>;
}
type NickelApplicationScalePolicy = Readonly<{policy:"follow"|"unchanged"}> | Readonly<{policy:"custom";scale_120:number}>;
interface NickelApplicationScaleSnapshot extends NickelAvailability {
    revision?:string; configured?:NickelApplicationScalePolicy; supported_scales?:ReadonlyArray<number>;
    native_per_monitor?:boolean; uncertain?:boolean;
    toolkits?:ReadonlyArray<Readonly<{family:"gtk"|"qt";available:boolean;live:boolean;restart_required:boolean;owned:boolean;pending:boolean}>>;
    last_result?:Readonly<{rejected?:boolean;uncertain?:boolean;refresh_required?:boolean;outcomes?:ReadonlyArray<Readonly<{family:"gtk"|"qt";kind:"unchanged"|"confirmed"|"external_conflict"|"unavailable"|"failed"|"uncertain";restart_required:boolean}>>}>;
}
interface NickelApplication { id:string;name:string;icon:string;pinned:boolean;pinOrder:number|null;recentOrder:number|null;kind:"place"|"application";launchClass:"graphical"|"terminal"|"running";canLaunch?:boolean;canPin?:boolean }
interface NickelApplicationSearch extends NickelAvailability {query:string;results:ReadonlyArray<Readonly<NickelApplication>>;total:number;truncated?:boolean;catalogTruncated?:boolean;nativeProjectsAvailable?:boolean;status?:string|null;pinSaveFailed?:boolean}
interface NickelNativeWindow {
    id:string;applicationId:string|null;title:string;active:boolean;
    minimized:boolean;maximized:boolean;fullscreen:boolean;workspace:string|null;output:string|null;
    canActivate:boolean;canClose:boolean;canMinimize:boolean;canMaximize:boolean;
    canFullscreen:boolean;canSnap:boolean;canMoveToWorkspace:boolean;canMoveToOutput:boolean;
}
interface NickelTrayItem {id:string;title:string;icon:boolean}
interface NickelNotification {id:number;appName:string;summary:string;body:string;actions:ReadonlyArray<Readonly<{key:string;label:string}>>}
interface NickelNotificationSnapshot {notification:Readonly<NickelNotification>|null;history:ReadonlyArray<Readonly<NickelNotification>>;visible:boolean}
/** Bounded native audio state; device identities/names are redacted while locked. */
interface NickelAudioSnapshot extends NickelAvailability {muted:boolean;percent:number;devices:ReadonlyArray<Readonly<{id:string;name:string;isDefault:boolean}>>}
interface NickelSessionSnapshot {revision:string;account:Readonly<{displayName:string;username:string}>|null;locked:boolean;support:Readonly<{lock:boolean;logout:boolean;suspend:boolean;reboot:boolean;powerOff:boolean;restartShell:boolean}>}
interface NickelPreferences {
    barOnAllDisplays:boolean;allWindowsOnEveryBar:boolean;desktopCount:number;
    preferredTerminal:string|null;preferredFileManager:string|null;fileIconProvider:"nickel"|"system";fileIconTheme:string|null;
    idleDimSeconds:number|null;idleLockSeconds:number|null;idleSuspendSeconds:number|null;
}
interface NickelPreferencesSnapshot extends NickelWritable {configured?:Readonly<NickelPreferences>;applications?:ReadonlyArray<Readonly<{id:string}>>;iconThemes?:ReadonlyArray<string>;unavailableSelections?:Readonly<{preferredTerminal:boolean;preferredFileManager:boolean;fileIconTheme:boolean}>}
interface NickelWifiNetwork {id:string;name:string;signalPercent:number;connected:boolean;saved:boolean;canConnect:boolean;canDisconnect:boolean}
interface NickelWifiSnapshot extends NickelWritable {enabled:boolean;adaptersAvailable:boolean;adapters:ReadonlyArray<Readonly<{name:string;description:string;connected:boolean;speedBitsPerSecond:number|null}>>;networks:ReadonlyArray<Readonly<NickelWifiNetwork>>;operations:Readonly<{setEnabled?:boolean;connect?:boolean;disconnect?:boolean}>}
interface NickelBluetoothDevice {id:string;name:string;paired:boolean;connected:boolean;batteryPercent:number|null;kind:string|null;signalDbm:number|null}
interface NickelBluetoothSnapshot extends NickelWritable {adapterName:string;powered:boolean;discovering:boolean;devices:ReadonlyArray<Readonly<NickelBluetoothDevice>>;operations:Readonly<{setPowered?:boolean;setDiscovery?:boolean;connect?:boolean;disconnect?:boolean;pair?:boolean}>}
interface NickelAssociationHandler {id:string;name:string;icon:string|null;source:string;protected:boolean}
interface NickelAssociationTarget {id:string;family:string;capability:"directUserChange"|"nativeConsent"|"readOnly"|"unsupported";scope:"user"|"system"|"policy";detail:string;protected:boolean;effectiveHandlerId:string|null;canSetDefault:boolean;handlersTruncated:boolean;handlers:ReadonlyArray<Readonly<NickelAssociationHandler>>}
interface NickelAssociationsSnapshot extends NickelWritable {targets:ReadonlyArray<Readonly<NickelAssociationTarget>>;truncated?:boolean;operations:Readonly<{setDefault?:boolean;openSystemSettings?:boolean}>;lastResult?:Readonly<{status:string;revision?:string;targetId?:string;handlerId?:string|null;detail?:string}>|null}

/** Local registrations retain their component and callback functions. */
interface NickelSettingMetadata {id:string;group:string;label:string;description?:string;order?:number;groupOrder?:number;requiredCapabilities?:ReadonlyArray<string>}
type NickelSettingControl =
    | {type:"switch";defaultValue:boolean}
    | {type:"slider"|"number";defaultValue:number;min:number;max:number;step?:number}
    | {type:"select";defaultValue:string;options:ReadonlyArray<Readonly<{value:string;label:string}>>}
    | {type:"color";defaultValue:string;allowAlpha?:boolean}
    | {type:"text";defaultValue:string;maxLength?:number;multiline?:boolean}
    | {type:"shortcut";defaultValue?:ReadonlyArray<string>}
    | {type:"action"}
    | {type:"group";fields:ReadonlyArray<NickelSettingField>}
    | {type:"repeated";fields:ReadonlyArray<NickelSettingField>;defaultValue?:ReadonlyArray<Readonly<Record<string,NickelJson>>>;minItems?:number;maxItems:number};
type NickelSettingField = {id:string;label:string;description?:string;order?:number} & NickelSettingControl;
type NickelSettingValue<C extends NickelSettingControl> = C extends {type:"switch"} ? boolean
    : C extends {type:"slider"|"number"} ? number
    : C extends {type:"select"|"color"|"text"} ? string
    : C extends {type:"shortcut"} ? ReadonlyArray<string>
    : C extends {type:"group"} ? Readonly<Record<string,NickelJson>>
    : C extends {type:"repeated"} ? ReadonlyArray<Readonly<Record<string,NickelJson>>> : NickelJson;
type NickelSettingRegistrationFor<C extends NickelSettingControl> = C extends NickelSettingControl
    ? NickelSettingMetadata & C & {value?:()=>NickelSettingValue<C>;onChange?:(value:NickelSettingValue<C>)=>void} : never;
type NickelSettingRegistration = NickelSettingRegistrationFor<NickelSettingControl>;
interface NickelSettingsPageRegistration extends NickelSettingMetadata {component:NickelComponent}
type NickelPublishedSetting = NickelSettingRegistration & {providerPackage:string};
type NickelPublishedSettingsPage = NickelSettingMetadata & {providerPackage:string;component:NickelComponent | Readonly<{module:string;export:string}>};
declare function registerSetting(definition:NickelSettingRegistration):void;
declare function registerSettingsPage(definition:NickelSettingsPageRegistration):void;
declare function readPluginSettings():Readonly<{generation:number;settings:ReadonlyArray<NickelPublishedSetting>}>;
declare function readSettingsPages():Readonly<{generation:number;pages:ReadonlyArray<NickelPublishedSettingsPage>}>;
/** Compatibility alias of readSettingsPages. */
declare function readPluginSettingsPages():ReturnType<typeof readSettingsPages>;

declare const nickel: Readonly<{
    readonly data: Readonly<Record<string, unknown> & {
        displays?: NickelDisplaySnapshot;
        appearance?:NickelAppearanceSnapshot;
        wallpaper?:NickelWallpaperSnapshot;
        preferences?:NickelPreferencesSnapshot;
        session?:NickelSessionSnapshot;
        audio?:NickelAudioSnapshot;
        windows?:ReadonlyArray<Readonly<NickelNativeWindow>>;
        applications?:ReadonlyArray<Readonly<NickelApplication>>;
        applicationSearch?:NickelApplicationSearch;
        notifications?:NickelNotificationSnapshot|null;
        tray?:ReadonlyArray<Readonly<NickelTrayItem>>;
        clock?:NickelClockSnapshot;
        workspaces?:NickelWorkspaceSnapshot;
        wifi?:NickelWifiSnapshot;
        bluetooth?:NickelBluetoothSnapshot;
        settings?: Readonly<Record<string, boolean | number | string>>;
        surface?: Readonly<{
            id: string;
            kind: "panel" | "dock" | "desktop" | "window" | "dialog" | "overlay";
            width: number;
            height: number;
        }>;
    }>;
    request(effect: NickelSurfaceRequest): void;
    request(effect: string | Readonly<{ type: string; [key: string]: unknown }>): void;
    openDialog(id: string): void;
    openMenu(id: string): void;
    /** Resolve a public callable component. Unknown contracts throw. Render it with ordinary JSX children. */
    component<P = NickelProps>(contract:string):NickelComponent<P>;
    /** Each entry supplies a callable component. An uncomposed package returns an empty collection. */
    contributions(collection:string):ReadonlyArray<NickelContribution>;
    registerSetting:typeof registerSetting;
    registerSettingsPage:typeof registerSettingsPage;
    readPluginSettings:typeof readPluginSettings;
    readSettingsPages:typeof readSettingsPages;
    readPluginSettingsPages:typeof readPluginSettingsPages;
    surfaces:Readonly<{show(id:string):void;hide(id:string):void;focus(id:string):void;setPlacement(id:string,placement:Readonly<{anchor:NickelAnchor;offsetX?:number;offsetY?:number}>):void}>;
    /** Requires appearance-read; set also requires appearance-control and a current observation. */
    appearance:Readonly<{get():NickelAppearanceSnapshot;/** Complete preferences, rather than a partial patch. */set(preferences:NickelAppearancePreferences):void}>;
    /** Requires wallpaper-read; mutations also require wallpaper-control. Image identities are opaque. */
    wallpaper:Readonly<{get():NickelWallpaperSnapshot;listImages():NickelWallpaperSnapshot["images"];setPosition(position:NickelWallpaperPosition):void;resetCustomImage():void;/** Opens the native picker; no paths or arguments are accepted. */chooseImage():void;selectImage(id:string):void}>;
    /** Requires windows-read; activation requires windows-focus, closing requires windows-context. */
    windowPreviews:Readonly<{
        get():Readonly<{available:boolean;open?:boolean;revision?:string;taskSwitcher?:boolean;windows:ReadonlyArray<Readonly<{id:string;title:string;applicationName:string;selected:boolean;canClose:boolean;canActivate:boolean;image:string}>>}>;
        activate(id:string,revision:string):void;
        close(id:string,revision:string):void;
        openMenu(id:string,revision:string):void;
    }>;
    windows:Readonly<{
        list():ReadonlyArray<Readonly<NickelNativeWindow>>;
        activate(id:string):void;close(id:string):void;
        menu():Readonly<{targetId:string|null;generation?:string}>;
        showMenu(id:string):void;dismissMenu(options?:Readonly<{restoreFocus?:boolean}>):void;
        minimize(id:string):void;maximize(id:string):void;restore(id:string):void;
        toggleMaximize(id:string):void;toggleFullscreen(id:string):void;
        snapLeading(id:string):void;snapTrailing(id:string):void;
        destinations():Readonly<{workspaces:ReadonlyArray<Readonly<{id:string;name:string}>>;outputs:ReadonlyArray<string>}>;
        moveToWorkspace(id:string,workspace:string):void;moveToOutput(id:string,output:string):void;
    }>;
    /** Requires applications-read; launch requires applications-launch; pin operations require applications-pin. */
    applications:Readonly<{list():ReadonlyArray<Readonly<NickelApplication>>;search(query:string):void;searchResults():NickelApplicationSearch;launch(id:string):void;togglePin(id:string):void;movePin(id:string,direction:-1|1):void;retryPinSave():void}>;
    /** Requires audio-read; native controls require audio-control. */
    audio:Readonly<{get():NickelAudioSnapshot;outputs():NickelAudioSnapshot["devices"];setVolume(percent:number):void;setMuted(muted:boolean):void;selectOutput(id:string):void}>;
    /** Requires notifications.read; invoke/dismiss also require notifications.act. */
    notifications:Readonly<{get():NickelNotificationSnapshot|null;invoke(id:number,key:string):void;dismiss(id:number):void}>;
    /** Requires tray-read; activation requires tray-activate; context menus require tray-context. */
    tray:Readonly<{list():ReadonlyArray<Readonly<NickelTrayItem>>;activate(id:string):void;contextMenu(id:string):void}>;
    session:Readonly<{get():NickelSessionSnapshot;lock():void;logout():void;suspend():void;reboot():void;powerOff():void;restartShell():void}>;
    /** Requires workspaces-read; mutations require workspaces-switch. IDs are opaque decimal strings. */
    workspaces:Readonly<{get():NickelWorkspaceSnapshot;switch(id:string):void;create():void;remove(id:string):void}>;
    /** Check operations before toggling native show-desktop; requires desktop-control. */
    desktop:Readonly<{get():NickelAvailability & {operations:Readonly<{toggleShowDesktop?:boolean}>};toggleShowDesktop():void}>;
    /** Requires run-command. */
    run:Readonly<{get():Readonly<{available:boolean;revision?:string;status:string|null}>;execute(command:string,expectedRevision?:string):void}>;
    /** Requires projects-menu-show. */
    projects:Readonly<{show():void;toggle():void}>;
    /** Requires on-screen-keyboard-show. */
    keyboard:Readonly<{
        get():Readonly<{available:boolean;generation:number;recipientAvailable:boolean;height?:number;dockTop?:boolean;rows:ReadonlyArray<ReadonlyArray<Readonly<{id:string;label:string;enabled:boolean;quarters:number}>>>;operations:Readonly<{press?:boolean;hide?:boolean;toggleDock?:boolean;holdModifiers?:boolean;resize?:boolean}>}>;
        toggle():void;press(id:string):void;hide():void;toggleDock():void;holdModifiers():void;resize(delta:-32|32):void;
    }>;
    /** Native wall-clock snapshot updates each minute. Format its values in JSX. */
    clock:Readonly<{get():Readonly<NickelClockSnapshot>}>;
    /** Requires preferences-read; set requires preferences-control. Unavailable choices are preserved until explicitly changed. */
    preferences:Readonly<{get():NickelPreferencesSnapshot;set(patch:Partial<NickelPreferences>):void}>;
    /** Requires network-read; controls require network-control and an available operation. */
    wifi:Readonly<{get():NickelWifiSnapshot;listNetworks():ReadonlyArray<Readonly<NickelWifiNetwork>>;setEnabled(enabled:boolean):void;connect(id:string):void;disconnect(id:string):void}>;
    /** Requires bluetooth-read; controls require bluetooth-control and an available operation. */
    bluetooth:Readonly<{get():NickelBluetoothSnapshot;listDevices():ReadonlyArray<Readonly<NickelBluetoothDevice>>;setPowered(powered:boolean):void;setDiscovery(discovering:boolean):void;connect(id:string):void;disconnect(id:string):void;pair(id:string):void}>;
    /** Requires associations-read; controls require associations-control and explicit current revision. */
    associations:Readonly<{get():NickelAssociationsSnapshot;list():ReadonlyArray<Readonly<NickelAssociationTarget>>;getHandlers(targetId:string):Readonly<{revision:string|undefined;target:Readonly<NickelAssociationTarget>;handlers:ReadonlyArray<Readonly<NickelAssociationHandler>>}>;setDefault(targetId:string,handlerId:string,expectedRevision:string):void;openSystemSettings():void}>;
    system: Readonly<{
        get():Readonly<{available:boolean;version:string|null;platform:string|null;architecture:string|null}>;
    }>;
    plugins: Readonly<{
        /** Requires plugins-read; unavailable memory counters are null. */
        get(): Readonly<{available:boolean; writable:boolean; revision?:string; truncated?:boolean; reason?:string; selectedShell?:string; shellPreview?:Readonly<{token:string;previousShell:string;selectedShell:string;deadlineUnixMilliseconds:number;canConfirm:boolean;canRevert:boolean}>|null; plugins:ReadonlyArray<NickelPluginStatus>; lastResult?:Readonly<{status:string;detail?:string}> | null}>;
        list(): ReadonlyArray<NickelPluginStatus>;
        /** Requires plugins-read and plugins-control and a current inventory revision. */
        enable(id:string, revision:string):void;
        disable(id:string, revision:string):void;
        setEnabled(id:string, enabled:boolean, revision:string):void;
        /** Start a timed preview; confirmation persists the selection. */
        selectShell(id:string, revision:string):void;
        confirmShell(token:string, revision:string):void;
        revertShell(token:string, revision:string):void;
        setSetting(id:string, key:string, value:boolean|number|string, revision:string):void;
    }>;
    /** Read-only reference: native shortcut remapping is currently unsupported. Requires shortcuts-read. */
    shortcuts: Readonly<{
        get(): Readonly<{available: boolean; editable: false; reason?: string; globalAvailable?: boolean; globalReason?: string | null; shortcuts: ReadonlyArray<Readonly<{id: string; action: string; keys: string; scope: string; available: boolean}>>}>;
    }>;
    /** Copied native feature snapshot; controls require features-control and its current revision. */
    features: Readonly<{
        get(): Readonly<{available: boolean; revision?: string; reason?: string; operations: Readonly<{setKeyboardMode?: boolean; setCodexEnabled?: boolean; retryCodex?: boolean}>; keyboard: Readonly<{mode?: "automatic" | "enabled" | "disabled"; generation?: number; environmentOverride?: boolean; runtimeAvailable?: boolean; enabled?: boolean | null; touchscreenPresent?: boolean | null}>; codex: Readonly<{configuredEnabled?: boolean; requestedEnabled?: boolean; generation?: number; acknowledgedGeneration?: number; state?: string; policy?: string; support?: string; installation?: string; health?: string; source?: string; diagnostic?: string | null; disableConfirmationRequired?: boolean; runtimeCountersAvailable?: boolean; activeWindows?: number | null; backgroundWorkers?: number | null; subscriptions?: number | null; warmSurfaces?: number | null; cacheEntries?: number | null}>; lastResult?: Readonly<{status: string; detail: string}> | null}>;
        setKeyboardMode(mode: "automatic" | "enabled" | "disabled"): void;
        setCodexEnabled(enabled: boolean, confirmed?: boolean): void;
        retryCodex(): void;
    }>;
    displays: Readonly<{
        /** Read a copy of the current host supplied display snapshot. */
        get(): NickelDisplaySnapshot | undefined;
        /** Preview a complete display layout; it automatically reverts after 15 seconds unless confirmed. */
        setLayout(layout: Readonly<NickelDisplayLayout>, expectedRevision?: string): void;
        getApplicationScale(): NickelApplicationScaleSnapshot;
        setApplicationScale(policy: NickelApplicationScalePolicy, expectedRevision?: string): void;
        /** Requires operations.identify; unsupported platforms reject the operation. */
        identify(expectedRevision?: string): void;
        /** Preview a supported projectionModes entry; confirm/revert finish its transaction. */
        previewProjection(mode:NickelProjectionMode):void;
        /** Keep the previewed layout. */
        confirm(): void;
        /** Restore the layout from before the preview. */
        revert(): void;
    }>;
}>;
