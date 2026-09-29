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

interface NickelProps {
    key?: string | number;
    className?: string;
    children?: NickelChild;
}

interface NickelDivProps extends NickelProps {
    id?: string;
    onClick?: NickelClick;
    role?: "button" | "radio" | "radiogroup" | "option";
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
    width: number | "100%";
    height: number | "100%";
    placement?: "managed" | "fixed";
    output?: "primary" | "all";
    edge?: "top" | "bottom" | "left" | "right";
    anchor?: "center" | "top-left" | "top-right" | "bottom-left" | "bottom-right";
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
    onContextMenu?: NickelClick;
    onDrag?: (gesture: NickelDragGesture) => void;
}
interface NickelTextFieldProps extends NickelProps {
    id?: string;
    value?: string;
    placeholder?: string;
    /** Mask the displayed value and protect the surface from remote inspection. */
    secure?: boolean;
    onChange: (value: string) => void;
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
interface NickelFileTileProps extends NickelProps {
    id: string;
    asset: string;
    label: string;
    x: number;
    y: number;
    width: number;
    height: number;
    selected?: boolean;
    hovered?: boolean;
    dragging?: boolean;
    color: NickelColor;
    outline: NickelColor;
    hoverBackground: NickelColor;
    selectedBackground: NickelColor;
    accent: NickelColor;
    complement: NickelColor;
    onClick?: NickelClick;
    onSelect?: NickelClick;
    onMove?: (delta: { dx: number; dy: number }) => void;
    onFileAction?: (request: { action: "cut" | "copy" | "rename" | "properties" | "open-terminal" }) => void;
}

declare function h(kind: unknown, props?: object | null, ...children: NickelChild[]): JSX.Element;
declare function Panel(props: NickelPanelProps): JSX.Element;
declare function Surface(props: NickelSurfaceProps): JSX.Element;
declare function Window(props: NickelWindowProps): JSX.Element;
/** Convenience JSX component that renders Window with fixed placement. */
declare function FixedWindow(props: Omit<NickelWindowProps, "placement">): JSX.Element;
declare function Viewport(props: NickelViewportProps): JSX.Element;
declare function Box(props: NickelBoxProps): JSX.Element;
/** Generic CSS layout box. Defaults to block layout. */
declare function Div(props: NickelDivProps): JSX.Element;
declare function FileTile(props: NickelFileTileProps): JSX.Element;
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
declare function Button(props: NickelButtonProps): JSX.Element;
declare function Spacer(props: NickelProps): JSX.Element;
declare function Dialog(props: NickelDialogProps): JSX.Element;
declare function Menu(props: NickelMenuProps): JSX.Element;
declare function MenuItem(props: NickelMenuItemProps): JSX.Element;

declare function useState<T>(initial: T | (() => T)): [T, (next: T | ((previous: T) => T)) => void];
declare function useRef<T>(initial: T): { current: T };

type NickelSurfaceRequest = Readonly<
    | { type: "show-plugin-surface"; surfaceId: string }
    | { type: "hide-plugin-surface"; surfaceId: string }
>;

declare const nickel: Readonly<{
    readonly data: Readonly<Record<string, unknown> & {
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
}>;
