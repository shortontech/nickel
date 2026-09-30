// @jsx h
import { arrangement, snapPlacement } from "./display-layout.js";
import "./styles/displays.css";

function ApplicationScaleControls() {
    const snapshot = nickel.displays.getApplicationScale();
    if (!snapshot.available) return <Text wrap={true}>{snapshot.reason || "Application UI scale is unavailable."}</Text>;
    const policy = snapshot.configured || {policy:"follow"};
    const custom = snapshot.supported_scales?.includes(policy.scale_120) ? policy.scale_120 : 120;
    const set = next => nickel.displays.setApplicationScale(next, snapshot.revision);
    const outcomes = snapshot.last_result?.outcomes || [];
    return <Column className="display-modes">
        <Text className="display-heading">Application UI scale</Text>
        {[{id:"follow",label:"Follow Nickel"},{id:"unchanged",label:"Leave application settings unchanged"},{id:"custom",label:"Custom scale"}].map(choice =>
            <Button key={choice.id} id={"application-scale-"+choice.id} state={policy.policy===choice.id?"selected":"unselected"}
                onClick={()=>set(choice.id==="custom"?{policy:"custom",scale_120:custom}:{policy:choice.id})}>{choice.label}</Button>)}
        {policy.policy==="custom" ? <Column>
            <Text>{Math.round(custom/1.2)+"%"}</Text>
            <Slider id="application-scale-value" accessibilityLabel="Application UI scale" min={60} max={480} step={30} value={custom}
                onChange={value=>set({policy:"custom",scale_120:value})}/>
        </Column>:null}
        {snapshot.native_per_monitor ? <Text wrap={true}>Windows manages native application DPI per display. This preference is saved for compatible applications.</Text>:null}
        {snapshot.toolkits?.some(toolkit=>!toolkit.available) ? <Text wrap={true}>Some application toolkits are unavailable on this system.</Text>:null}
        {outcomes.some(outcome=>outcome.kind==="confirmed" && outcome.restart_required) ? <Text wrap={true}>Restart affected applications to use the new scale.</Text>:null}
        {snapshot.uncertain ? <Text wrap={true}>An application scale change could not be confirmed. Refresh before trying again.</Text>:null}
        {outcomes.some(outcome=>outcome.kind==="external_conflict") ? <Text wrap={true}>Some application settings changed elsewhere and were left unchanged.</Text>:null}
        {outcomes.some(outcome=>outcome.kind==="failed") || snapshot.last_result?.rejected ? <Text wrap={true}>The application scale change could not be completed.</Text>:null}
    </Column>;
}

export function Displays() {
    const snapshot = nickel.displays.get() || {available:false,outputs:[],reason:"Display capability is unavailable."};
    const outputs = snapshot.outputs || [];
    const [selectedName, select] = useState(null);
    const draftState = useRef(null);
    const setDraft = next => { draftState.current = next; };
    const drag = useRef(null);
    const revision = snapshot.revision || JSON.stringify(outputs);
    if (draftState.current && draftState.current.revision !== revision) draftState.current = null;
    const draft = draftState.current ? draftState.current.outputs : outputs;
    const selected = draft.find(output => output.name === selectedName) || draft[0];
    const view = arrangement(draft);
    const layout = next => ({primary:next.find(output => output.enabled && output.primary)?.name || next.find(output => output.enabled)?.name || "",
        placements:next.map(output => ({name:output.name,x:output.geometry.x,y:output.geometry.y,enabled:output.enabled,scale_120:output.scale_120,mode:output.current_mode,transform:output.transform}))});
    const update = (name, patch) => setDraft({revision,outputs:draft.map(output => output.name === name ? {...output,...patch} : output)});
    const onDrag = (output, gesture) => {
        if (gesture.phase === "start") {drag.current={name:output.name,x:gesture.x,y:gesture.y}; return;}
        if (gesture.phase === "cancel") {drag.current=null; return;}
        const start = drag.current;
        if (gesture.phase !== "end" || !start || start.name !== output.name) return;
        drag.current=null;
        const moved={...output,geometry:{...output.geometry,x:output.geometry.x+Math.round((gesture.x-start.x)/view.scale),y:output.geometry.y+Math.round((gesture.y-start.y)/view.scale)}};
        const placement=snapPlacement(moved,draft);
        update(output.name,{geometry:{...moved.geometry,...placement}});
    };
    if (!snapshot.available) return <Column><Text wrap={true}>{snapshot.reason || "Display control is unavailable."}</Text><ApplicationScaleControls/></Column>;
    if (!selected) return <Column><Text>No displays are available.</Text><ApplicationScaleControls/></Column>;
    return <Column className="display-page">
        <Row><Text className="display-heading">Arrange displays</Text><Button id="display-identify" disabled={!snapshot.operations?.identify} onClick={()=>nickel.displays.identify(snapshot.revision)}>Identify</Button></Row>
        <Text wrap={true}>Drag displays to match their physical positions. Apply to preview your changes.</Text>
        <Layer id="display-arrangement" className="display-arrangement">
            {view.cards.map((card,index) => <Box key={card.name} x={card.x} y={card.y} width={card.width} height={card.height}>
                <Button id={"display-card-"+index} width={card.width} height={card.height}
                    className={selected.name === card.name ? "display-card selected" : "display-card"}
                    accessibilityLabel={(card.model || card.name)+" display, "+card.name}
                    state={selected.name === card.name ? "selected" : "unselected"}
                    onDrag={gesture=>onDrag(card,gesture)} onClick={()=>select(card.name)}>
                    {(card.model || card.name)+(card.primary?" · Primary":"")+(!card.enabled?" · Disabled":"")}
                </Button>
            </Box>)}
        </Layer>
        <Text className="display-heading">{selected.model || selected.name}</Text>
        <Row><Text>Enabled</Text><Switch id="display-enabled" accessibilityLabel="Enable display" state={selected.enabled?"on":"off"}
            onClick={()=>update(selected.name,{enabled:!selected.enabled})}/></Row>
        <Text>Resolution and refresh rate</Text>
        <Column className="display-modes">
            {(selected.modes || []).map((mode,index)=><Button key={mode.width+"x"+mode.height+"@"+mode.refresh_millihz} id={"display-mode-"+index}
                state={JSON.stringify(mode)===JSON.stringify(selected.current_mode)?"selected":"unselected"}
                onClick={()=>update(selected.name,{current_mode:mode})}>
                {mode.width+" × "+mode.height+" · "+(mode.refresh_millihz/1000).toFixed(2)+" Hz"}
            </Button>)}
        </Column>
        {snapshot.operations?.setOrientation ? <Column className="display-modes">
            <Text>Orientation</Text>
            {[{id:"normal",label:"Landscape"},{id:"rotate90",label:"Portrait"},{id:"rotate180",label:"Landscape flipped"},{id:"rotate270",label:"Portrait flipped"}].map(orientation => <Button key={orientation.id}
                id={"display-orientation-"+orientation.id} state={selected.transform===orientation.id?"selected":"unselected"}
                onClick={()=>update(selected.name,{transform:orientation.id})}>{orientation.label}</Button>)}
        </Column>:null}
        <Text>{"Display scale: "+Math.round(selected.scale_120/1.2)+"%"}</Text>
        <Slider id="display-scale" accessibilityLabel="Display scale" min={Math.min(60,selected.scale_120)} max={Math.max(480,selected.scale_120)} step={12} value={selected.scale_120}
            onChange={value=>update(selected.name,{scale_120:value})}/>
        <Row className="display-actions">
            <Button id="display-primary" disabled={!selected.enabled} onClick={()=>setDraft({revision,outputs:draft.map(output=>({...output,primary:output.name===selected.name}))})}>Make primary</Button>
            <Button id="display-apply" disabled={!draft.some(output=>output.enabled)} onClick={()=>nickel.displays.setLayout(layout(draft), snapshot.revision)}>Apply</Button>
            <Button id="display-discard" onClick={()=>setDraft(null)}>Discard draft</Button>
        </Row>
        {snapshot.pending_confirmation ? <Row className="display-actions">
            <Text>Keep display settings?</Text>
            <Button id="display-keep" disabled={snapshot.can_confirm === false} onClick={()=>nickel.displays.confirm()}>Keep</Button>
            <Button id="display-revert" disabled={snapshot.can_revert === false} onClick={()=>nickel.displays.revert()}>Revert</Button>
        </Row>:null}
        <ApplicationScaleControls/>
    </Column>;
}
registerSettingsPage({id:"displays",group:"System",label:"Displays",description:"Arrange displays and change their modes and scale",component:Displays});
