// Nickel's global JSX API. This file is for editors and the development
// compiler; the installed plugin still contains plain JavaScript. Components
// are declared callable for JSX checking; their runtime values are tag strings.
declare namespace JSX {
    interface Element {}
    interface ElementChildrenAttribute { children: {} }
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
    children?: NickelChild;
}

interface NickelPanelProps extends NickelProps {
    background?: NickelColor;
    height?: number;
}
interface NickelSurfaceProps extends NickelProps {
    width: number;
    height: number;
    background?: NickelColor;
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
    accessibilityLabel?: string;
    icon?: string;
    showLabel?: boolean;
    onClick: NickelClick;
    onContextMenu?: NickelClick;
    onDrag?: (gesture: NickelDragGesture) => void;
}
interface NickelTextFieldProps extends NickelProps {
    id: string;
    value?: string;
    placeholder?: string;
    onChange: (value: string) => void;
}
interface NickelImageProps extends NickelProps {
    asset: string;
    width: number;
    height: number;
    fit?: "contain" | "cover" | "stretch";
}
interface NickelImageButtonProps extends NickelImageProps {
    id: string;
    accessibilityLabel: string;
    onClick: NickelClick;
    onContextMenu?: NickelClick;
}
interface NickelDialogProps extends NickelProps {
    id?: string;
    anchor: string;
    open?: boolean;
    width?: number;
    height?: number;
}
interface NickelMenuProps extends NickelProps {
    id: string;
    anchor: string;
    open?: boolean;
}
interface NickelMenuItemProps extends NickelProps {
    id: string;
    onClick: NickelClick;
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
interface NickelFileTileProps extends NickelProps {
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
}

declare function h(kind: unknown, props?: object | null, ...children: NickelChild[]): JSX.Element;
declare function Panel(props: NickelPanelProps): JSX.Element;
declare function Surface(props: NickelSurfaceProps): JSX.Element;
declare function Box(props: NickelBoxProps): JSX.Element;
declare function FileTile(props: NickelFileTileProps): JSX.Element;
declare function Badge(props: NickelBadgeProps): JSX.Element;
declare function Row(props: NickelProps): JSX.Element;
declare function Column(props: NickelProps): JSX.Element;
declare function ScrollView(props: NickelProps & { id: string; height?: number }): JSX.Element;
declare function Text(props: NickelTextProps): JSX.Element;
declare function Image(props: NickelImageProps): JSX.Element;
declare function ImageButton(props: NickelImageButtonProps): JSX.Element;
declare function Progress(props: NickelProgressProps): JSX.Element;
declare function TextField(props: NickelTextFieldProps): JSX.Element;
declare function Button(props: NickelButtonProps): JSX.Element;
declare function Spacer(props: NickelProps): JSX.Element;
declare function Dialog(props: NickelDialogProps): JSX.Element;
declare function Menu(props: NickelMenuProps): JSX.Element;
declare function MenuItem(props: NickelMenuItemProps): JSX.Element;

declare function useState<T>(initial: T | (() => T)): [T, (next: T | ((previous: T) => T)) => void];
declare function useRef<T>(initial: T): { current: T };

declare const nickel: Readonly<{
    readonly data: Readonly<Record<string, unknown> & {
        settings?: Readonly<Record<string, boolean | number | string>>;
    }>;
    request(effect: string): void;
    openDialog(id: string): void;
    openMenu(id: string): void;
}>;
