/// <reference path="../../crates/twinkle-jsx-runtime/types/twinkle.d.ts" />
// Nickel's global JSX API. This file is for editors and the development
// compiler; the installed plugin still contains plain JavaScript. Components
// and public component references are callable in JSX. Children are ordinary JSX values.
declare function useWindows(): ReadonlyArray<Readonly<NickelNativeWindow>>;
declare function useWindows<T>(selector: (windows: ReadonlyArray<Readonly<NickelNativeWindow>>) => T): T;
declare function useActiveWindow(): Readonly<NickelNativeWindow> | null;
declare function useWindowPreviews(): ReturnType<typeof nickel.windowPreviews.get>;
declare function useWindowPreviews<T>(selector: (snapshot: ReturnType<typeof nickel.windowPreviews.get>) => T): T;
declare function useWindowMenu(): ReturnType<typeof nickel.windows.menu>;
declare function useWindowMenu<T>(selector: (snapshot: ReturnType<typeof nickel.windows.menu>) => T): T;
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
interface NickelWifiSnapshot extends NickelWritable {enabled:boolean;adaptersAvailable:boolean;adapters:ReadonlyArray<Readonly<{id:string;name:string;description:string;connected:boolean;speedBitsPerSecond:number|null}>>;networks:ReadonlyArray<Readonly<NickelWifiNetwork>>;operations:Readonly<{setEnabled?:boolean;connect?:boolean;disconnect?:boolean}>}
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

type NickelChild = TwinkleChild;
type NickelComponent<P=NickelProps> = TwinkleComponent<P>;
type NickelAnchor = TwinkleAnchor;
type NickelJson = TwinkleJson;
type NickelColor = TwinkleColor;
type NickelClick = TwinkleClick;
type NickelDragGesture = TwinkleDragGesture;
type NickelDropGesture = TwinkleDropGesture;
type NickelProps = TwinkleProps;
type NickelDivProps = TwinkleDivProps;
type NickelPanelProps = TwinklePanelProps;
type NickelWindowProps = TwinkleWindowProps;
type NickelBoxProps = TwinkleBoxProps;
type NickelTextProps = TwinkleTextProps;
type NickelButtonProps = TwinkleButtonProps;
type NickelTextFieldProps = TwinkleTextFieldProps;
type NickelSliderProps = TwinkleSliderProps;
type NickelSwitchProps = TwinkleSwitchProps;
type NickelColorSwatchProps = TwinkleColorSwatchProps;
type NickelSelectProps = TwinkleSelectProps;
type NickelOptionProps = TwinkleOptionProps;
type NickelImageProps = TwinkleImageProps;
type NickelImageButtonProps = TwinkleImageButtonProps;
type NickelDialogProps = TwinkleDialogProps;
type NickelMenuProps = TwinkleMenuProps;
type NickelMenuItemProps = TwinkleMenuItemProps;
type NickelBadgeProps = TwinkleBadgeProps;
type NickelProgressProps = TwinkleProgressProps;
type NickelContext<T> = TwinkleContext<T>;
type NickelLocaleSnapshot = TwinkleLocaleSnapshot;
interface NickelExternalStore<T> extends TwinkleExternalStore<T> {}
type NickelComponentFailure = TwinkleComponentFailure;
type NickelErrorBoundaryProps = TwinkleErrorBoundaryProps;
type NickelThemePalette = TwinkleThemePalette;
type NickelThemeSnapshot = TwinkleThemeSnapshot;
type NickelSurfaceSnapshot = TwinkleSurfaceSnapshot;
declare function useOutput():NickelDisplaySnapshot["outputs"][number]|null;
