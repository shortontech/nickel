// @jsx h
import { fromHsv, toHexColor } from "./colors.js";
import "./styles/appearance.css";

const defaults = {theme:"system", accent_hue:null, accent_intensity:null, reduce_transparency:false, animations:"normal"};
const wallpaperKey = image => image.id;
// Estimates only: native layout corrects these for the current font, width, and scale.
const wallpaperHeight = image => image.previewAsset ? 126 : 36;

function AppearanceSlider({value, max, onCommit, ...props}) {
    const [draft, setDraft] = useState(null);
    const drag = useRef(false);
    const committed = useRef(value);
    const base = useRef(value);
    if (base.current !== value) { base.current = value; committed.current = value; }
    const commit = next => {
        if (committed.current !== next) { committed.current = next; onCommit(next); }
        setDraft(null);
    };
    return <Slider {...props} min={0} max={max} step={1}
        value={draft && draft.base === value ? draft.value : value}
        onChange={next => drag.current ? setDraft({base:value, value:next}) : commit(next)}
        onDrag={gesture => {
            if (gesture.phase === "start") drag.current = true;
            if (gesture.phase === "end") {
                drag.current = false;
                commit(Math.round(Math.max(0, Math.min(1, (gesture.x - gesture.bounds.x) / Math.max(1, gesture.bounds.width))) * max));
            }
            if (gesture.phase === "cancel") { drag.current = false; setDraft(null); }
        }} />;
}

export function Appearance() {
    const appearance = nickel.appearance.get();
    const wallpaper = nickel.wallpaper.get();
    const [customOpen, setCustomOpen] = useState(false);
    const [customHue, setCustomHue] = useState("");
    if (!appearance.available) return <Text wrap={true}>{appearance.reason || "Appearance is unavailable."}</Text>;
    const configured = appearance.configured;
    const hue = configured.accent_hue ?? appearance.resolved.hue;
    const intensity = configured.accent_intensity ?? appearance.resolved.intensity;
    const set = patch => nickel.appearance.set({...configured, ...patch});
    const modes = [{id:"light", label:"Light"}, {id:"dark", label:"Dark"}, {id:"system", label:"Automatic"}];
    const hueNumber = Number(customHue);
    const validHue = customHue.trim() !== "" && Number.isInteger(hueNumber) && hueNumber >= 0 && hueNumber <= 359;
    return <Column className="appearance-page">
        {!appearance.writable ? <Text>Appearance is read only.</Text> : null}
        <Column className="appearance-card">
            <Text className="appearance-heading">Theme</Text>
            <Row className="appearance-choices">
                {modes.map(mode => <Button key={mode.id} id={"appearance-mode-" + mode.id}
                    className={configured.theme === mode.id ? "appearance-choice selected" : "appearance-choice"}
                    state={configured.theme === mode.id ? "selected" : "unselected"}
                    disabled={!appearance.writable} onClick={() => set({theme:mode.id})}>{mode.label}</Button>)}
            </Row>
        </Column>
        <Column className="appearance-card">
            <Text className="appearance-heading">Accent color</Text>
            <Row className="appearance-swatches">
                {[0,30,60,120,180,210,270,330].map(value => <ColorSwatch key={value}
                    id={"appearance-accent-" + value} color={toHexColor(fromHsv([value,0.65,0.85,1]))}
                    selected={hue === value} accessibilityLabel={"Accent hue " + value + " degrees"}
                    onClick={() => { if (appearance.writable) set({accent_hue:value}); }} />)}
                <Button id="appearance-accent-custom" disabled={!appearance.writable}
                    onClick={() => {setCustomHue(String(hue)); setCustomOpen(true);}}>Custom hue</Button>
            </Row>
            <Text>{"Hue: " + hue + "°"}</Text>
            {appearance.writable ? <AppearanceSlider id="appearance-hue" accessibilityLabel="Interface hue"
                max={359} value={hue} onCommit={value => set({accent_hue:value})} /> : null}
            <Text>{"Color intensity: " + intensity + "%"}</Text>
            {appearance.writable ? <AppearanceSlider id="appearance-intensity" accessibilityLabel="Color intensity"
                max={100} value={intensity} onCommit={value => set({accent_intensity:value})} /> : null}
        </Column>
        <Column className="appearance-card">
            <Text className="appearance-heading">Interface</Text>
            <Row><Text>Reduce transparency</Text>
                <Switch id="appearance-transparency" accessibilityLabel="Reduce transparency"
                    state={appearance.writable ? configured.reduce_transparency ? "on" : "off" : configured.reduce_transparency ? "disabled-on" : "disabled-off"}
                    onClick={appearance.writable ? () => set({reduce_transparency:!configured.reduce_transparency}) : undefined} />
            </Row>
            <Text>Animations</Text>
            <Row className="appearance-choices">
                {["off","reduced","normal"].map(value => <Button key={value} id={"appearance-animation-" + value}
                    state={configured.animations === value ? "selected" : "unselected"}
                    className={configured.animations === value ? "appearance-choice selected" : "appearance-choice"}
                    disabled={!appearance.writable} onClick={() => set({animations:value})}>{value}</Button>)}
            </Row>
        </Column>
        <Column className="appearance-card">
            <Text className="appearance-heading">Wallpaper</Text>
            {!wallpaper.available ? <Text wrap={true}>{wallpaper.reason || "Wallpaper is unavailable."}</Text> : <Column>
                <Text>{wallpaper.configured.custom_image_configured ? "Custom image" : "Default wallpaper"}</Text>
                <div className="appearance-wallpaper-positions">
                    {["center","tile","stretch","fit","span","fill"].map(position => <Button key={position}
                        id={"appearance-wallpaper-position-" + position} disabled={!wallpaper.writable}
                        state={wallpaper.configured.position === position ? "selected" : "unselected"}
                        onClick={() => nickel.wallpaper.setPosition(position)}>{position}</Button>)}
                </div>
                <Button id="appearance-wallpaper-choose" disabled={!wallpaper.writable || !wallpaper.chooser?.available || wallpaper.chooser.pending}
                    onClick={() => nickel.wallpaper.chooseImage()}>{wallpaper.chooser?.pending ? "Choosing image…" : "Choose image…"}</Button>
                {wallpaper.chooser?.result ? <Text wrap={true}>{wallpaper.chooser.result.reason || ({applied:"Wallpaper applied",cancelled:"Image choice cancelled"})[wallpaper.chooser.result.status] || ""}</Text> : null}
                <VirtualColumn id="appearance-wallpapers" items={wallpaper.images}
                    itemKey={wallpaperKey} itemHeight={wallpaperHeight} overscan={96}
                    renderItem={(image,index) => <Column key={image.id}>
                    {image.previewAsset ? <Image asset={image.previewAsset} width={160} height={90} fit="cover" /> : null}
                    <Button key={image.id} id={"appearance-wallpaper-" + image.id}
                    disabled={!wallpaper.writable} onClick={() => nickel.wallpaper.selectImage(image.id)}>
                    {(image.label || "Wallpaper " + (index + 1)) + (image.configured ? " · Current" : "")}</Button></Column>} />
                <Button id="appearance-wallpaper-remove" disabled={!wallpaper.writable}
                    onClick={() => nickel.wallpaper.resetCustomImage()}>Use default wallpaper</Button>
            </Column>}
        </Column>
        <Button id="appearance-reset" disabled={!appearance.writable} onClick={() => nickel.appearance.set(defaults)}>Reset appearance</Button>
        <Dialog id="appearance-custom-hue-dialog" anchor="appearance-accent-custom" open={customOpen}
            width={360} height={216} onClose={() => setCustomOpen(false)}>
            <Column className="appearance-card">
                <Text className="appearance-heading">Custom hue</Text>
                <Text>Choose a hue from 0 to 359 degrees.</Text>
                <TextField id="appearance-custom-hue-input" accessibilityLabel="Custom hue" value={customHue} onChange={setCustomHue} />
                <Row className="appearance-choices">
                    <Button id="appearance-custom-hue-apply" disabled={!validHue || !appearance.writable}
                        onClick={() => {set({accent_hue:hueNumber}); setCustomOpen(false);}}>Apply</Button>
                    <Button id="appearance-custom-hue-cancel" onClick={() => setCustomOpen(false)}>Cancel</Button>
                </Row>
            </Column>
        </Dialog>
    </Column>;
}

registerSettingsPage({id:"appearance", group:"Personalization", label:"Appearance", description:"Color theme, accent, interface, and wallpaper", component:Appearance});
