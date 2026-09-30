// Nickel's global JSX API. This file is for editors and the development
// compiler; the installed plugin still contains plain JavaScript. Components
// are declared callable for JSX checking; their runtime values are tag strings.
declare namespace JSX {
    interface Element {}
    interface ElementChildrenAttribute { children: {} }
    interface IntrinsicElements {
        div: NickelDivProps;
    }
}

type NickelChild = JSX.Element | string | number | null | false | NickelChild[];
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
interface NickelTextProps extends NickelProps { color?: NickelColor }
interface NickelButtonProps extends NickelProps {
    id?: string;
    width?: number;
    height?: number;
    accessibilityLabel?: string;
    icon?: string;
    showLabel?: boolean;
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
    onChange: (fraction: number) => void;
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
interface NickelWidgetProps extends NickelProps {
    label: string;
    value: string;
    percent: number;
    color?: NickelColor;
}
interface NickelActionProps extends NickelProps {
    id: string;
    item?: string;
    label: string;
    onClick: (applicationId: string) => void;
}
interface NickelSectionProps extends NickelProps {
    id: string;
    label: string;
    value: string;
    onClick: () => void;
}
interface NickelProgressProps extends NickelProps {
    percent: number;
    width: number;
    height: number;
}
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
declare function Widget(props: NickelWidgetProps): JSX.Element;
declare function Action(props: NickelActionProps): JSX.Element;
declare function Section(props: NickelSectionProps): JSX.Element;
declare function Row(props: NickelProps): JSX.Element;
declare function Column(props: NickelProps): JSX.Element;
declare function ScrollView(props: NickelProps & { id: string; height?: number; grow?: boolean }): JSX.Element;
declare function Text(props: NickelTextProps): JSX.Element;
declare function Image(props: NickelImageProps): JSX.Element;
declare function ImageButton(props: NickelImageButtonProps): JSX.Element;
declare function Progress(props: NickelProgressProps): JSX.Element;
declare function TextField(props: NickelTextFieldProps): JSX.Element;
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
declare function useRef<T>(initial: T): { current: T };

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
        scale_120: number;
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

interface NickelDisplaySnapshot {
    available: boolean;
    reason?: string;
    outputs: ReadonlyArray<Readonly<{
        name: string;
        model: string;
        geometry: Readonly<{ x: number; y: number; width: number; height: number }>;
        work_area: Readonly<{ x: number; y: number; width: number; height: number }>;
        scale_120: number;
        transform: "normal" | "rotate90" | "rotate180" | "rotate270"
            | "flipped" | "flipped90" | "flipped180" | "flipped270";
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

declare const nickel: Readonly<{
    readonly data: Readonly<Record<string, unknown> & {
        displays?: NickelDisplaySnapshot;
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
    system: Readonly<{
        get():Readonly<{available:boolean;version:string|null;platform:string|null;architecture:string|null}>;
    }>;
    plugins: Readonly<{
        /** Requires plugins-read; unavailable memory counters are null. */
        get(): Readonly<{available:boolean; writable:boolean; revision?:string; selectedShell?:string; plugins:ReadonlyArray<NickelPluginStatus>; lastResult?:Readonly<{status:string;detail?:string}> | null}>;
        list(): ReadonlyArray<NickelPluginStatus>;
        /** Requires plugins-read and plugins-control and a current inventory revision. */
        enable(id:string, revision:string):void;
        disable(id:string, revision:string):void;
        setEnabled(id:string, enabled:boolean, revision:string):void;
        selectShell(id:string, revision:string):void;
        setSetting(id:string, key:string, value:boolean|number|string, revision:string):void;
    }>;
    /** Read-only reference: native shortcut remapping is currently unsupported. Requires shortcuts-read. */
    shortcuts: Readonly<{
        get(): Readonly<{available: boolean; editable: false; reason?: string; globalAvailable?: boolean; globalReason?: string | null; shortcuts: ReadonlyArray<Readonly<{id: string; action: string; keys: string; scope: string; available: boolean}>>}>;
    }>;
    /** Copied native feature snapshot; controls require features-control and capture its current revision. */
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
        setLayout(layout: Readonly<NickelDisplayLayout>): void;
        /** Keep the previewed layout. */
        confirm(): void;
        /** Restore the layout from before the preview. */
        revert(): void;
    }>;
}>;
