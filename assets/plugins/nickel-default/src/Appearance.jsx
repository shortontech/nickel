// @jsx h
import { fromHsv, toHexColor } from "./colors.js";
import "./styles/appearance.css";

const defaults = {theme:"system", accent_hue:null, accent_intensity:null, reduce_transparency:false, animations:"normal"};

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
            {appearance.writable ? <Slider id="appearance-hue" accessibilityLabel="Interface hue"
                min={0} max={359} step={1} value={hue} onChange={value => set({accent_hue:value})} /> : null}
            <Text>{"Color intensity: " + intensity + "%"}</Text>
            {appearance.writable ? <Slider id="appearance-intensity" accessibilityLabel="Color intensity"
                min={0} max={100} step={1} value={intensity} onChange={value => set({accent_intensity:value})} /> : null}
            <Button id="appearance-inherit-accent" disabled={!appearance.writable}
                onClick={() => set({accent_hue:null,accent_intensity:null})}>Use system accent</Button>
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
                <Row className="appearance-choices">
                    {["center","tile","stretch","fit","span","fill"].map(position => <Button key={position}
                        id={"appearance-wallpaper-position-" + position} disabled={!wallpaper.writable}
                        state={wallpaper.configured.position === position ? "selected" : "unselected"}
                        onClick={() => nickel.wallpaper.setPosition(position)}>{position}</Button>)}
                </Row>
                {wallpaper.images.map((image,index) => <Button key={image.id} id={"appearance-wallpaper-" + image.id}
                    disabled={!wallpaper.writable} onClick={() => nickel.wallpaper.selectImage(image.id)}>
                    {"Wallpaper " + (index + 1) + (image.configured ? " · Current" : "")}</Button>)}
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

registerSettingsPage({id:"appearance", group:"Personalization", label:"Appearance", description:"Theme, accent, interface, and wallpaper", component:Appearance});
